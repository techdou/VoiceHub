// 「语音识别服务」列表的持久化，以及与运行时配置的同步。
//
// 两套键，职责必须分清（与 aiProfileStore 同一套思路）：
//   1. 列表本身 —— `cloudAsr.profiles` + `cloudAsr.activeProfileId`，只有设置页读写；
//   2. 运行时生效的那一份 —— `cloudAsr.provider / apiKey / appId`，外加
//      `cloudAsr.qwen.workspaceId`（流式字幕判定要用）与 `cloudAsr.omniSystemPrompt`。
//      这些扁平键由录音链路（CloudAPIProvider、RecorderOrchestrator）、历史记录重跑、
//      识别测试、诊断页共同消费，**这个契约不能动**。
//
// 规则：列表是真相，每次列表或启用项变化都把启用的那一份整份写进运行时键。
// 「切了供应商但密钥还是上一家的」这类 bug 就是因为过去这些键各自维护；现在只有一个写入点。
//
// profiles 里带密钥，会跟着配置导出走（Rust 侧 export_app_settings 是整表导出，无需登记新键）。

import { getSetting, setSetting } from '@/services/store'
import { DOUBAO_KEYS, resolveDoubaoConsole } from '@/lib/cloudAsrCreds'
import {
  ASR_PLATFORMS,
  ASR_PROVIDERS,
  effectiveAsrCredentials,
  emptyAsrProfile,
  parseAsrProfiles,
  resolveActiveAsrProfile,
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
 * 把启用的那份写进运行时键。
 *
 * workspaceId 与 omniPrompt 也要跟着走：它们分别决定「能不能开流式字幕」和
 * 「Omni 怎么整理」，如果只在设置页里改而不同步，运行时用的还是上一份的值。
 * 没有可用服务时一律写空串，让下游自己判定未配置。
 */
async function syncRuntimeActive(profile: AsrProfile | null): Promise<void> {
  const creds = profile ? effectiveAsrCredentials(profile) : { apiKey: '', appId: '' }
  await Promise.all([
    setSetting('cloudAsr.provider', profile?.provider ?? ''),
    setSetting('cloudAsr.apiKey', creds.apiKey),
    setSetting('cloudAsr.appId', creds.appId),
    setSetting('cloudAsr.apiUrl', profile?.apiUrl?.trim() ?? ''),
    setSetting('cloudAsr.model', profile?.model?.trim() ?? ''),
    setSetting('cloudAsr.qwen.workspaceId', profile?.workspaceId?.trim() ?? ''),
    setSetting('cloudAsr.omniSystemPrompt', profile?.omniPrompt ?? ''),
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
 * 按已有凭据补齐档案：**平台填了密钥，就为该平台下的每个服务各建一份**。
 *
 * 为什么是「每个服务」而不是「每个平台一份」：改成档案列表之前，这一页摆的是内置清单，
 * 6 个服务不管配没配都看得见。如果只按平台建一份，千问平台下 4 个变体
 * （一次性 / 流式 / 两个 Omni）就只剩 1 张卡，用户升级后会发现"我的服务少了 3 个"——
 * 而它们本来都是能用的（共用同一把百炼 key）。升级不该悄悄拿走东西。
 *
 * 两条跳过规则，缺一不可：
 *  · 已有同 provider 的档案 —— 不重复建；
 *  · 已在 alreadyAuto 里 —— 之前自动补过、被用户删掉了，不该再"救回来"。
 *
 * 返回 added 让调用方记账，这也是它能反复安全执行的原因。
 */
export function topUpProfiles(
  existing: AsrProfile[],
  credsByPlatform: Partial<Record<AsrPlatform, PlatformCreds>>,
  alreadyAuto: string[] = [],
): { profiles: AsrProfile[]; added: string[] } {
  const added: AsrProfile[] = []
  for (const entry of ASR_PROVIDERS) {
    const creds = credsByPlatform[entry.platform]
    if (!creds || !hasKey(creds)) continue
    if (existing.some((p) => p.provider === entry.id)) continue
    if (alreadyAuto.includes(entry.id)) continue
    const profile = emptyAsrProfile(entry.id)
    profile.apiKey = creds.apiKey
    profile.otherKey = creds.otherKey
    profile.appId = creds.appId
    profile.console = creds.console
    profile.workspaceId = creds.workspaceId
    profile.omniPrompt = entry.omni ? creds.omniPrompt : ''
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

  let profiles = parseAsrProfiles(rawProfiles)
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
    // 先记账再落盘：这一步失败也只会导致下次重来，不会造成"补过了却没记"的空档
    await setSetting(ASR_AUTO_CREATED_KEY, [...alreadyAuto, ...topUp.added])
  }

  // 启用项：没有记录时沿用运行时那个 provider 对应的档案（升级前用的就是它）
  if (!activeId && profiles.length > 0) {
    const legacyProvider = await getSetting('cloudAsr.provider', 'doubao_v2') as string
    activeId = profiles.find((p) => p.provider === legacyProvider)?.id ?? profiles[0].id
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

  return { profiles, activeId }
}

/** 落盘列表 + 启用项，并同步运行时键。所有写路径都必须走这里。 */
export async function saveAsrProfiles(state: AsrProfileState): Promise<void> {
  const active = resolveActiveAsrProfile(state.profiles, state.activeId)
  await Promise.all([
    setSetting(ASR_PROFILES_KEY, state.profiles),
    setSetting(ASR_ACTIVE_PROFILE_KEY, active?.id ?? ''),
  ])
  await syncRuntimeActive(active)
}
