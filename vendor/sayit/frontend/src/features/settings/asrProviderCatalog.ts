// 云 API 语音识别的供应商目录、服务档案（profile）解析与延迟分档。
//
// 结构与「AI 服务」页对齐：一张卡 = 一份完整配置（供应商 + 该平台的凭据），可以存多份，
// 同一家也能存多份（比如两个百炼账号、两套豆包密钥）。启用哪一份由 activeProfileId 决定。
//
// 和 AI 服务的唯一实质差别：供应商只能从内置清单里选，不能填任意地址 —— 每家 ASR 的
// 协议都要一份专门的 Rust 实现（asr_doubao / asr_qwen / asr_qwen_omni / asr_mimo），
// 不像 AI 整理那边只要是 OpenAI 兼容端点就能接。
//
// 「同平台重复粘密钥」的问题不靠"按平台存"解决（那样一张卡就不再是一份完整配置了），
// 而是照 AI 服务的做法：新建时若该平台已有配置，自动把密钥带过来，界面上说明它从哪来。

import {
  describeDoubaoMissing,
  effectiveDoubaoCredentials,
  type DoubaoConsole,
  type DoubaoCredentials,
} from '@/lib/cloudAsrCreds'

/** 凭据归属的平台。同平台的服务用同一种密钥形态。 */
import { t } from '@/i18n'

export type AsrPlatform = 'doubao' | 'qwen' | 'mimo' | 'groq' | 'custom'

export interface AsrPlatformInfo {
  label: string
  /** 去哪儿领密钥 */
  consoleUrl: string
}

// label 用 getter：这是模块级常量，普通字段会把加载那一刻的语言固化下来。
// 见 aiProviderCatalog.ts 里 AiProvider.label 的注释。
export const ASR_PLATFORMS: Record<AsrPlatform, AsrPlatformInfo> = {
  custom: { label: 'OpenAI compatible ASR', consoleUrl: '' },
  doubao: { get label() { return t('asrPlatform.doubao') }, consoleUrl: 'https://console.volcengine.com/speech/app' },
  qwen: { get label() { return t('asrPlatform.qwen') }, consoleUrl: 'https://bailian.console.aliyun.com' },
  mimo: { get label() { return t('asrPlatform.mimo') }, consoleUrl: 'https://xiaoai.mi.com' },
  groq: { label: 'Groq', consoleUrl: 'https://console.groq.com/keys' },
}

export interface AsrProviderEntry {
  /** 存进 cloudAsr.provider 的值，也是 Rust 侧分发用的 key */
  id: string
  /** 卡片标题，短 */
  label: string
  /** 实际模型名，小字 */
  model: string
  platform: AsrPlatform
  /** 一句定位，说清「什么时候选它」 */
  blurb: string
  /**
   * 当前内置端点与控制台所面向的账号地区。
   * `global` 是海外端点（Groq），国内直连可能不稳定，卡片上要如实标出来 ——
   * 否则用户会把「连不上」当成自己密钥填错。
   */
  availability: 'mainland_china' | 'global'
  /** 边说边出字（实时字幕） */
  streaming?: boolean
  /** 识别与 AI 整理一步完成，不需要单独配 AI 服务 */
  omni?: boolean
  /** 需要百炼「业务空间 ID」才能用 */
  needsWorkspaceId?: boolean
}

/**
 * 内置服务清单。顺序即卡片顺序，也是新建时下拉的顺序：把最推荐的放前面。
 *
 * label 刻意不带模型名后缀（模型名单独一行小字），否则卡片标题会被
 * 「千问 3.5 Omni Plus（qwen3.5-omni-plus，ASR+AI）」这种括号串撑爆。
 *
 * ── blurb 的口径（改文案前先读这段）──
 * 每条只回答一件事：**什么时候该选它**，而且七条放在一起要能相互区分。
 * 早期的文案是各写各的优点（「准确率高、速度快」同时挂在两家上），等于没给出
 * 选择依据 —— 用户看完还是不知道该点哪一张。现在的口径：
 *   · 准确率的结论是**有条件的**，条件要写出来。豆包只有在**关掉实时字幕**时
 *     才是中文最准的一档，一开字幕就掉档；Audio 3.0 反过来，开着字幕也是第一档。
 *     少了这个前提，两张卡的「最准」会互相矛盾。
 *   · 不如别家的地方照实写（千问 realtime 不及 Audio 3.0、MiMo 准确率一般），
 *     但用词留余地 —— 目录里留着它们是因为有人的账号/网络只有那一家能用。
 *   · 引用别的卡一律用**产品名**（"Audio 3.0"、"Omni Plus"），不要用「上面那个」
 *     「同上」：顺序会变，位置指代改一次顺序就错。
 *   · 这些准确率结论来自实际使用对比，不是跑分。改结论前先真用一遍。
 */
export const ASR_PROVIDERS: AsrProviderEntry[] = [
  {
    id: 'doubao_v2',
    get label() { return t('asrProvider.doubao') },
    model: 'Doubao-Seed-ASR-2.0',
    platform: 'doubao',
    availability: 'mainland_china',
    get blurb() { return t('asrProvider.doubaoBlurb') },
  },
  // 第二位（豆包之后）：实测**开着实时字幕**时它比豆包更准 —— 豆包最准的用法是
  // 关掉实时字幕，一开就掉一档；这一家开着字幕也是第一档。它也不需要业务空间 ID
  // （通用域名可用，见 providers/asr_qwen_audio_stream.rs 的文件头），
  // 所以是"填了密钥就能用"的那张卡。
  {
    id: 'qwen_audio_stream',
    get label() { return t('asrProvider.qwenAudioStream') },
    model: 'qwen-audio-3.0-asr-flash-streaming',
    platform: 'qwen',
    availability: 'mainland_china',
    get blurb() { return t('asrProvider.qwenAudioStreamBlurb') },
    streaming: true,
  },
  {
    id: 'qwen',
    get label() { return t('asrProvider.qwen') },
    model: 'qwen3-asr-flash',
    platform: 'qwen',
    availability: 'mainland_china',
    get blurb() { return t('asrProvider.qwenBlurb') },
  },
  {
    id: 'qwen_realtime',
    get label() { return t('asrProvider.qwenRealtime') },
    model: 'qwen3-asr-flash-realtime',
    platform: 'qwen',
    availability: 'mainland_china',
    get blurb() { return t('asrProvider.qwenRealtimeBlurb') },
    streaming: true,
    needsWorkspaceId: true,
  },
  {
    id: 'qwen_omni_35_plus',
    get label() { return t('asrProvider.omniPlus') },
    model: 'qwen3.5-omni-plus',
    platform: 'qwen',
    availability: 'mainland_china',
    get blurb() { return t('asrProvider.omniPlusBlurb') },
    omni: true,
  },
  {
    id: 'qwen_omni_35_flash',
    get label() { return t('asrProvider.omniFlash') },
    model: 'qwen3.5-omni-flash',
    platform: 'qwen',
    availability: 'mainland_china',
    get blurb() { return t('asrProvider.omniFlashBlurb') },
    omni: true,
  },
  {
    id: 'mimo',
    get label() { return t('asrProvider.mimo') },
    model: 'mimo-v2.5-asr',
    platform: 'mimo',
    availability: 'mainland_china',
    get blurb() { return t('asrProvider.mimoBlurb') },
  },
  {
    id: 'groq_whisper',
    get label() { return t('asrProvider.groq') },
    model: 'whisper-large-v3-turbo',
    platform: 'groq',
    availability: 'global',
    get blurb() { return t('asrProvider.groqBlurb') },
  },
  { id: 'openai_compat', label: '自定义 ASR / OpenAI 兼容', model: 'Custom model', platform: 'custom', blurb: '本地服务或第三方转写接口', availability: 'mainland_china' },
]

export function findAsrProvider(id: string): AsrProviderEntry | undefined {
  return ASR_PROVIDERS.find((p) => p.id === id)
}

/**
 * 地区提示，**只在需要提醒时才给字**。
 *
 * `mainland_china` 刻意返回空串：内置清单里绝大多数都是国内端点，那是默认情形，
 * 每张卡上都挂一句「面向中国大陆账号」等于把噪音重复七遍，还把真正要紧的
 * 「这个要海外网络才连得上」冲淡了。availability 字段本身保留 —— 它是数据，
 * 只是这一条不值得占界面上的字。
 */
export function asrAvailabilityLabel(entry: AsrProviderEntry): string {
  switch (entry.availability) {
    case 'global': return t('asrProvider.regionGlobal')
    default: return ''
  }
}

/** 同一平台下有哪些可选服务 */
export function providersOfPlatform(platform: AsrPlatform): AsrProviderEntry[] {
  return ASR_PROVIDERS.filter((p) => p.platform === platform)
}

// ─────────────────────────── 检测结果 ───────────────────────────

/**
 * 一次识别测试的结论。
 *
 * 存的是结论而不只是耗时：只有耗时的话，界面上能说出「多快」，说不出「行不行」。
 * 不可用也要留痕，否则用户点完测试、切走再回来，只看到一张和没测过一样的卡。
 */
export interface AsrCheck {
  ok: boolean
  /** 测试时间戳，用来判断结论是否还新鲜 */
  at: number
  /** 转写这段测试音频花了多久（毫秒），只有测通才有 */
  latencyMs?: number
  /** 测试音频时长（秒）。延迟要除以它才能横向比较，所以一起存 */
  audioSec?: number
  /** 不可用时的原因，一句话 */
  reason?: string
}

function isFiniteNumber(v: unknown): number | undefined {
  return typeof v === 'number' && Number.isFinite(v) ? v : undefined
}

function parseCheck(raw: unknown): AsrCheck | undefined {
  if (!raw || typeof raw !== 'object') return undefined
  const v = raw as Record<string, unknown>
  if (typeof v.ok !== 'boolean') return undefined
  const at = isFiniteNumber(v.at)
  if (at === undefined) return undefined
  return {
    ok: v.ok,
    at,
    latencyMs: isFiniteNumber(v.latencyMs),
    audioSec: isFiniteNumber(v.audioSec),
    reason: typeof v.reason === 'string' && v.reason ? v.reason : undefined,
  }
}

// ─────────────────────────── 服务档案 ───────────────────────────

/**
 * 一份语音识别服务配置 = 选定的供应商 + 该平台需要的凭据。
 *
 * 凭据直接挂在档案上（而不是按平台共享），这样一张卡就是一份自洽的完整配置，
 * 也才可能同时存两个账号。重复粘密钥的问题由「新建时自动沿用同平台上一份」解决。
 */
export interface AsrProfile {
  id: string
  /** 供应商 id，必须是 ASR_PROVIDERS 里的一个 */
  provider: string
  /** 千问 / 小米：API Key；豆包：当前控制台代次那一份密钥 */
  apiKey: string
  /** 豆包旧版控制台的 App ID */
  appId: string
  /** 豆包控制台代次 */
  console: DoubaoConsole
  /** 豆包另一代的密钥，切换代次时原样保留，不用重新粘 */
  otherKey: string
  /** 百炼业务空间 ID（流式识别需要） */
  workspaceId: string
  /** Omni 模型的 System Prompt（识别与整理一体，属于这份服务的行为） */
  omniPrompt: string
  apiUrl?: string
  model?: string
  /** 上次识别测试的结论 */
  check?: AsrCheck
}

export function makeAsrProfileId(): string {
  return `asr_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 8)}`
}

/** 一份空白档案。provider 默认给最推荐的那个。 */
export function emptyAsrProfile(provider = ASR_PROVIDERS[0].id): AsrProfile {
  return {
    id: makeAsrProfileId(),
    provider,
    apiKey: '',
    appId: '',
    console: 'new',
    otherKey: '',
    workspaceId: '',
    omniPrompt: '',
  }
}

function str(v: unknown): string {
  return typeof v === 'string' ? v : ''
}

/**
 * 容错解析存储里的档案列表。
 *
 * 存储可能被手改过、也可能是更早版本写的，坏条目一律丢弃而不是让整页崩掉。
 * provider 不在内置清单里的条目也丢掉 —— 留着只会渲染出一张点不动的卡。
 */
export function parseAsrProfiles(raw: unknown): AsrProfile[] {
  if (!Array.isArray(raw)) return []
  const out: AsrProfile[] = []
  const seen = new Set<string>()
  for (const item of raw) {
    if (!item || typeof item !== 'object') continue
    const v = item as Record<string, unknown>
    const provider = str(v.provider)
    if (!findAsrProvider(provider)) continue
    const id = str(v.id) || makeAsrProfileId()
    if (seen.has(id)) continue
    seen.add(id)
    const consoleRaw = str(v.console)
    out.push({
      id,
      provider,
      apiKey: str(v.apiKey),
      appId: str(v.appId),
      console: consoleRaw === 'legacy' ? 'legacy' : 'new',
      otherKey: str(v.otherKey),
      workspaceId: str(v.workspaceId),
      omniPrompt: str(v.omniPrompt),
      ...(provider === 'openai_compat' ? { apiUrl: str(v.apiUrl), model: str(v.model) } : {}),
      check: parseCheck(v.check),
    })
  }
  return out
}

/** 启用项归一化：指向不存在的 id 时回落到第一条 */
export function resolveActiveAsrProfile(
  profiles: AsrProfile[],
  activeId: string,
): AsrProfile | null {
  return profiles.find((p) => p.id === activeId) ?? profiles[0] ?? null
}

/**
 * 档案里的豆包字段 → lib/cloudAsrCreds 的凭据结构。
 *
 * 抽出来是因为两处都要用（算生效凭据、判断缺什么）。档案里只存「当前代次那把密钥」
 * 加「另一代那把」，映射时按代次还原成 consoleKey / accessToken。
 */
function toDoubaoCreds(profile: AsrProfile): DoubaoCredentials {
  return {
    console: profile.console,
    consoleKey: profile.console === 'new' ? profile.apiKey : profile.otherKey,
    accessToken: profile.console === 'new' ? profile.otherKey : profile.apiKey,
    appId: profile.appId,
  }
}

/**
 * 算出一份档案「本次生效的凭据」。
 *
 * 豆包新版控制台下 appId 必须是空串 —— Rust 侧就是靠它区分两代鉴权头，
 * 这条不变量的说明与测试在 lib/cloudAsrCreds.ts。
 */
export function effectiveAsrCredentials(profile: AsrProfile): { apiKey: string; appId: string } {
  if (findAsrProvider(profile.provider)?.platform === 'doubao') {
    return effectiveDoubaoCredentials(toDoubaoCreds(profile))
  }
  return { apiKey: profile.apiKey.trim(), appId: '' }
}

/**
 * 这份档案还缺什么才能用（空串 = 齐了）。文案直接可展示。
 *
 * 豆包那套直接委托 describeDoubaoMissing，不再自己抄一遍：抄的那版曾经把
 * 「还没填 API Key」写成没有空格的「还没填API Key」，而同一句话会和左下角就绪指示
 * 同屏出现，差一个空格就露馅。同一句用户可见文案只该有一个出处。
 */
export function describeAsrMissing(profile: AsrProfile): string {
  if (profile.provider === 'openai_compat') {
    if (!profile.model?.trim()) return '请填写 ASR 模型名称'
    try {
      const url = new URL(profile.apiUrl ?? '')
      if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) return '请填写有效的 HTTP(S) 接口地址'
    } catch { return '请填写有效的 HTTP(S) 接口地址' }
    return ''
  }
  if (findAsrProvider(profile.provider)?.platform === 'doubao') {
    return describeDoubaoMissing(toDoubaoCreds(profile))
  }
  return profile.apiKey.trim() ? '' : t('asrProvider.missingKey')
}

/**
 * 密钥尾巴，用来区分同一家的多份配置。
 *
 * 同供应商的两张卡在界面上只差凭据，不给个区分标记就成了两张一模一样的卡，
 * 用户不知道哪张是哪个账号。只露最后 4 位：够区分，又不算把密钥显示出来。
 */
export function keyFingerprint(profile: AsrProfile): string {
  const key = effectiveAsrCredentials(profile).apiKey
  return key.length >= 4 ? `••${key.slice(-4)}` : ''
}

// ─────────────────────────── 延迟分档 ───────────────────────────

/**
 * 为什么不直接复用 AI 服务那套 gradeLatency：口径完全不同。
 *
 * 那边测的是一次极小的 LLM 往返，1 秒就算慢。这边测的是**把一段音频转写成文字**，
 * 一段 3 秒的话花 1 秒返回是正常水平。照搬阈值会把所有 ASR 服务标成「太慢」。
 *
 * 所以按 RTF（耗时 ÷ 音频时长）分档，界面上仍显示毫秒数（用户看得懂的是这个），
 * 但结论词来自 RTF —— 这样即使将来换了测试音频，档位也不会跟着漂。
 */
export type AsrLatencyTier = 'instant' | 'fast' | 'normal' | 'slow' | 'tooSlow'

export interface AsrLatencyGrade {
  tier: AsrLatencyTier
  label: string
  /** ok = 放心用，warn = 能用但拖，bad = 不建议用于口述 */
  tone: 'ok' | 'warn' | 'bad'
}

/** RTF 分界值集中在这里，想调手感只改这一处 */
const RTF_THRESHOLDS = { instant: 0.15, fast: 0.3, normal: 0.5, slow: 0.8 } as const

export function gradeAsrLatency(latencyMs: number, audioSec: number): AsrLatencyGrade {
  // 音频时长缺失或不合理时不硬猜档位，只说「已测通」，避免给出一个凭空的结论
  if (!Number.isFinite(audioSec) || audioSec <= 0) {
    return { tier: 'normal', label: t('grade.tested'), tone: 'ok' }
  }
  const rtf = latencyMs / (audioSec * 1000)
  if (rtf < RTF_THRESHOLDS.instant) return { tier: 'instant', label: t('grade.instant'), tone: 'ok' }
  if (rtf < RTF_THRESHOLDS.fast) return { tier: 'fast', label: t('grade.fast'), tone: 'ok' }
  if (rtf < RTF_THRESHOLDS.normal) return { tier: 'normal', label: t('grade.normal'), tone: 'ok' }
  if (rtf < RTF_THRESHOLDS.slow) return { tier: 'slow', label: t('grade.slow'), tone: 'warn' }
  return { tier: 'tooSlow', label: t('grade.tooSlow'), tone: 'bad' }
}
