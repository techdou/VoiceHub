// 「语音识别服务」列表的持久化，以及与运行时配置的同步。
//
// 两套键，职责必须分清（与 aiProfileStore 同一套思路）：
//   1. 列表本身 —— `cloudAsr.profiles` + `cloudAsr.activeProfileId`，只有设置页读写；
//   2. 运行时生效的那一份 —— `cloudAsr.provider / model / apiKey / appId`，外加
//      `cloudAsr.qwen.workspaceId`（流式字幕判定要用）与 `cloudAsr.omniSystemPrompt`。
//      这些扁平键由录音链路（CloudAPIProvider、RecorderOrchestrator）、历史记录重跑、
//      识别测试、诊断页共同消费，**这个契约不能动**。
//
// 规则：列表是真相，每次列表或启用项变化都把启用的那一份整份写进运行时键。
// 「切了供应商但密钥还是上一家的」这类 bug 就是因为过去这些键各自维护；现在只有一个写入点。
//
// profiles 里带密钥，会跟着配置导出走（Rust 侧 export_app_settings 是整表导出，无需登记新键）。

import { getSetting, setSetting } from '@/services/store'
import { addRuntimeEvent } from '@/services/debugLog'
import { DOUBAO_KEYS, resolveDoubaoConsole } from '@/lib/cloudAsrCreds'
import {
  ASR_PLATFORMS,
  ASR_PROVIDERS,
  asrCardIdOfLegacyProvider,
  asrEndpointUrl,
  asrModelsOf,
  effectiveAsrCredentials,
  emptyAsrProfile,
  parseAsrProfilesDetailed,
  resolveActiveAsrProfile,
  resolveAsrApiModel,
  resolveAsrModelOption,
  resolveAsrRuntimeProvider,
  type AsrPlatform,
  type AsrProfile,
} from './asrProviderCatalog'

export const ASR_PROFILES_KEY = 'cloudAsr.profiles'
export const ASR_ACTIVE_PROFILE_KEY = 'cloudAsr.activeProfileId'
/**
 * 已经自动补过哪些服务（provider id 列表）。
 *
 * 为什么不用「迁移完成」这种布尔标记：那种标记只要被置上就再也不会重来，而它极容易
 * 在工作没真正做完时就被消耗掉 —— 开发期就踩过一次：热重载装载了改到一半的代码，
 * 迁移分支因为一个过期的前置条件被整段跳过，标记却照样置成了 true，于是修好之后的
 * 正确逻辑永远没机会执行，用户的服务一直少 3 个。
 *
 * 改成记录「补过谁」之后：
 *  · 缺哪个补哪个，天然幂等，不怕重复执行；
 *  · 用户主动删掉的卡不会被"救回来"（它已在列表里）；
 *  · 逻辑本身有 bug 修好后能自愈，不需要再造一个 v3 键。
 */
export const ASR_AUTO_CREATED_KEY = 'cloudAsr.autoCreatedProviders'

export interface AsrProfileState {
  profiles: AsrProfile[]
  activeId: string
}

/**
 * 本次加载时解析不出来的原始条目。落盘时原样拼回去。
 *
 * 为什么需要它：`parseAsrProfiles` 丢掉一个条目之后，下一次「归一化后写回」就会把它
 * 从磁盘上永久删除。而这条链子已经真的造成过损失 —— 迁移判据写错时，用户的
 * Gemini 与 OpenRouter 两张卡连密钥一起消失，而密钥在存储里没有第二份。
 *
 * 有了这份留存，即使解析逻辑再出错，原始 JSON 也一直在 db 里，改好代码下次加载即恢复。
 */
let orphanProfiles: unknown[] = []

/**
 * 把启用的那份写进运行时键。
 *
 * workspaceId 与 omniPrompt 也要跟着走：它们分别决定「能不能开流式字幕」和
 * 「Omni 怎么整理」，如果只在设置页里改而不同步，运行时用的还是上一份的值。
 * 没有可用服务时一律写空串，让下游自己判定未配置。
 *
 * `cloudAsr.model` 写的是**解析后**的模型名（不是档案里那个可能为空的原始值）：
 * 运行时链路只会把它原样传给后端，回落规则不该在四个组装点各实现一遍。
 */
async function syncRuntimeActive(profile: AsrProfile | null): Promise<void> {
  const creds = profile ? effectiveAsrCredentials(profile) : { apiKey: '', appId: '' }
  await Promise.all([
    // ⚠️ 写的是**选中模型的 provider**，不是 profile.provider（那是卡片 id = 平台）。
    // 一张卡里的模型可以走不同协议，真正的分发 key 只能从模型上取 ——
    // 写成卡片 id 的话 Rust 会报 "ASR provider not implemented"。
    setSetting('cloudAsr.provider', profile ? resolveAsrRuntimeProvider(profile) : ''),
    setSetting('cloudAsr.model', profile ? resolveAsrApiModel(profile) : ''),
    setSetting('cloudAsr.apiKey', creds.apiKey),
    setSetting('cloudAsr.appId', creds.appId),
    setSetting('cloudAsr.qwen.workspaceId', profile?.workspaceId?.trim() ?? ''),
    setSetting('cloudAsr.omniSystemPrompt', profile?.omniPrompt ?? ''),
    // 自定义端点地址。经 asrEndpointUrl 取值，所以只有 customEndpoint 的卡会写出
    // 非空值 —— 换成内置卡之后这里会被清成空串，不会把音频发到上一张卡的地址去。
    setSetting('cloudAsr.baseUrl', profile ? asrEndpointUrl(profile) : ''),
    // 「OpenAI 兼容」那张卡的协议。auto 也照原样存，由 buildAsrExtra 决定要不要下传。
    setSetting('cloudAsr.protocol', profile?.protocol ?? 'auto'),
  ])
}

/** 某个平台已保存的凭据（迁移时从旧的按平台键里读出来） */
interface PlatformCreds {
  apiKey: string
  otherKey: string
  appId: string
  console: 'new' | 'legacy'
  workspaceId: string
  omniPrompt: string
}

async function readPlatformCreds(platform: AsrPlatform): Promise<PlatformCreds> {
  const omniPrompt = await getSetting('cloudAsr.omniSystemPrompt', '') as string
  if (platform === 'doubao') {
    const appId = await getSetting(DOUBAO_KEYS.appId, '') as string
    const accessToken = await getSetting(DOUBAO_KEYS.accessToken, '') as string
    const consoleKey = await getSetting(DOUBAO_KEYS.consoleKey, '') as string
    const mode = resolveDoubaoConsole(await getSetting(DOUBAO_KEYS.console, '') as string, appId)
    return {
      apiKey: mode === 'new' ? consoleKey : accessToken,
      otherKey: mode === 'new' ? accessToken : consoleKey,
      appId,
      console: mode,
      workspaceId: '',
      omniPrompt,
    }
  }
  return {
    apiKey: await getSetting(`cloudAsr.${platform}.apiKey`, '') as string,
    otherKey: '',
    appId: '',
    console: 'new',
    workspaceId: platform === 'qwen'
      ? await getSetting('cloudAsr.qwen.workspaceId', '') as string
      : '',
    omniPrompt,
  }
}

function hasKey(creds: PlatformCreds): boolean {
  return creds.apiKey.trim() !== '' || creds.otherKey.trim() !== ''
}

/**
 * 按已有凭据补齐档案：**平台填了密钥，就给这个平台建一张卡**。
 *
 * 这条规则变过一次，两次的理由都记下来免得再绕回去：
 *   · 最早是「每个平台一份」，但那时**一张卡 = 一个模型**，于是千问平台下 4 个变体
 *     只剩 1 张卡，用户升级后发现"我的服务少了 3 个"——而它们本来都能用；
 *   · 于是改成「每个服务各建一份」，代价是界面上千问一家占 5 张卡；
 *   · 现在一张卡带模型下拉，一张卡就覆盖了该平台全部模型，所以回到「每平台一张」——
 *     这次不会丢东西，模型都在下拉里。
 *
 * 两条跳过规则，缺一不可：
 *  · 已有同一张卡的档案 —— 不重复建；
 *  · 已在 alreadyAuto 里 —— 之前自动补过、被用户删掉了，不该再"救回来"。
 *    存量记的是旧分发 key，要先换算成卡片 id（asrCardIdOfLegacyProvider），
 *    不换算的话老用户删掉的卡会在升级后回来一次。
 *
 * 返回 added 让调用方记账，这也是它能反复安全执行的原因。
 */
export function topUpProfiles(
  existing: AsrProfile[],
  credsByPlatform: Partial<Record<AsrPlatform, PlatformCreds>>,
  alreadyAuto: string[] = [],
): { profiles: AsrProfile[]; added: string[] } {
  const added: AsrProfile[] = []
  const autoCards = new Set(alreadyAuto.map(asrCardIdOfLegacyProvider))
  for (const entry of ASR_PROVIDERS) {
    const creds = credsByPlatform[entry.platform]
    if (!creds || !hasKey(creds)) continue
    if (existing.some((p) => p.provider === entry.id)) continue
    if (autoCards.has(entry.id)) continue
    const profile = emptyAsrProfile(entry.id)
    profile.apiKey = creds.apiKey
    profile.otherKey = creds.otherKey
    profile.appId = creds.appId
    profile.console = creds.console
    profile.workspaceId = creds.workspaceId
    // 这个平台有 Omni 模型就把 System Prompt 带上 —— 它是 profile 级字段，
    // 只在用户真选了 Omni 模型时才生效，先备着不会有副作用。
    profile.omniPrompt = asrModelsOf(entry).some((m) => m.omni) ? creds.omniPrompt : ''
    added.push(profile)
  }
  return {
    // 顺序跟着 ASR_PROVIDERS，保证卡片排列和内置清单一致
    profiles: [...existing, ...added],
    added: added.map((p) => p.provider),
  }
}

/**
 * 加载服务列表。首次进入会把旧的「按平台一套凭据」摊平成列表。
 *
 * 返回前会归一化 activeId（指向已不存在的 id 时回落到第一条），并在确实写过东西时
 * 顺手同步运行时键 —— 否则会出现「列表里高亮着 A，实际在用 B」。
 */
export async function loadAsrProfiles(): Promise<AsrProfileState> {
  const [rawProfiles, storedActiveId, rawAuto] = await Promise.all([
    getSetting(ASR_PROFILES_KEY, [] as unknown[]) as Promise<unknown>,
    getSetting(ASR_ACTIVE_PROFILE_KEY, '') as Promise<string>,
    getSetting(ASR_AUTO_CREATED_KEY, [] as unknown[]) as Promise<unknown>,
  ])

  const parsed = parseAsrProfilesDetailed(rawProfiles)
  // 解析不出来的条目原样记下来，落盘时拼回去。**绝不静默删除** ——
  // 一次解析 bug 曾经把用户的服务配置连密钥一起从磁盘上抹掉（密钥没有第二份）。
  orphanProfiles = parsed.orphans
  if (orphanProfiles.length > 0) {
    addRuntimeEvent('warn', 'settings', 'ASR profiles could not be parsed; kept as-is in storage', {
      count: orphanProfiles.length,
    })
  }
  let profiles = parsed.profiles
  let activeId = storedActiveId
  let needsWrite = false
  const alreadyAuto = Array.isArray(rawAuto) ? rawAuto.filter((x): x is string => typeof x === 'string') : []

  // 每次加载都按已有凭据补一次缺的服务。因为记的是「补过谁」而不是「补完了」，
  // 反复执行是安全的：已有的不动、删过的不回来。
  const credsByPlatform: Partial<Record<AsrPlatform, PlatformCreds>> = {}
  for (const platform of Object.keys(ASR_PLATFORMS) as AsrPlatform[]) {
    credsByPlatform[platform] = await readPlatformCreds(platform)
  }
  const topUp = topUpProfiles(profiles, credsByPlatform, alreadyAuto)
  if (topUp.added.length > 0) {
    profiles = topUp.profiles
    needsWrite = true
    // 先记账再落盘：这一步失败也只会下次重来，不会造成"补过了却没记"的空档
    await setSetting(ASR_AUTO_CREATED_KEY, [...alreadyAuto, ...topUp.added])
  }

  // VoiceHub 0.2.x 专属迁移：我们 0.2.0 嵌入版私有「自定义 ASR / OpenAI 兼容」通道，
  // 地址存的是运行时键 cloudAsr.apiUrl（上游从来没有过这个键，上游迁移链不认）。
  // 档案里已有 openai_compat 卡的不重建（parseAsrProfiles 对旧档案的 apiUrl 字段名
  // 与本版一致，直接兼容），但运行时键 baseUrl 体系是 0.2.2 新增的——旧升级路径上
  // needsWrite 可能为 false，不落一次盘的话 syncRuntimeActive 永不执行，新录音链
  // 只读 cloudAsr.baseUrl，最终报 missing_base_url。所以：旧键存在 → 无论是否补建
  // 档案都强制走一次 saveAsrProfiles（写全运行时键），成功后**消费旧键**（清空），
  // 防止迁移非一次性——用户删掉自定义卡后旧地址不会带着当前服务凭据复活。
  const legacyApiUrl = ((await getSetting('cloudAsr.apiUrl', '')) as string).trim()
  if (legacyApiUrl) {
    if (!profiles.some((p) => p.provider === 'openai_compat')) {
      const legacyModel = (await getSetting('cloudAsr.model', '')) as string
      const legacyApiKey = (await getSetting('cloudAsr.apiKey', '')) as string
      profiles.push({
        ...emptyAsrProfile('openai_compat'),
        name: 'VoiceHub 自定义通道',
        apiUrl: legacyApiUrl,
        model: legacyModel || 'whisper-1',
        apiKey: legacyApiKey,
      })
    }
    needsWrite = true
  }

  // 启用项：没有记录时沿用运行时那个 provider 对应的档案（升级前用的就是它）。
  // 运行时键存的是旧分发 key（`qwen_audio_stream`），要换算成卡片 id 才找得到。
  if (!activeId && profiles.length > 0) {
    const legacyProvider = await getSetting('cloudAsr.provider', 'doubao_v2') as string
    const card = asrCardIdOfLegacyProvider(legacyProvider)
    activeId = profiles.find((p) => p.provider === card)?.id ?? profiles[0].id
    needsWrite = true
  }

  const active = resolveActiveAsrProfile(profiles, activeId)
  if (active && active.id !== activeId) {
    activeId = active.id
    needsWrite = true
  }
  if (!active && activeId !== '') {
    activeId = ''
    needsWrite = true
  }

  if (needsWrite) await saveAsrProfiles({ profiles, activeId })

  // 迁移持久化成功后才消费旧键：下次加载 legacyApiUrl 为空，迁移不再触发，
  // 用户删除自定义卡也不会被旧地址复活（避免旧地址配新凭据发出去）。
  if (legacyApiUrl) await setSetting('cloudAsr.apiUrl', '')

  return { profiles, activeId }
}

/**
 * 落盘列表 + 启用项，并同步运行时键。所有写路径都必须走这里。
 *
 * ⚠️ 写入时会把 `orphanProfiles` 拼回去，所以**必须先经过 loadAsrProfiles**
 * （既有约定，activeId 也依赖它）。直接调这个函数会把解析不出来的条目丢掉。
 */
export async function saveAsrProfiles(state: AsrProfileState): Promise<void> {
  const active = resolveActiveAsrProfile(state.profiles, state.activeId)
  await Promise.all([
    setSetting(ASR_PROFILES_KEY, [...state.profiles, ...orphanProfiles]),
    setSetting(ASR_ACTIVE_PROFILE_KEY, active?.id ?? ''),
  ])
  await syncRuntimeActive(active)
}
