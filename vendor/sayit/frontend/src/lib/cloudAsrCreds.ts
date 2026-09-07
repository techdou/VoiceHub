// 云 API 模式的凭据归属与豆包控制台代次判定。
//
// 为什么单独一个模块：这些规则原来散在 CloudAPISection 与 stores/modeStatus 两处，
// 后者还留着一句「与 CloudAPISection.asrKeyGroup 保持一致」的注释——靠注释同步的东西
// 迟早会不一致。豆包新增控制台代次后要同步的规则又多一条，所以先收成一处。
import { t } from '@/i18n'

/** 豆包控制台代次。火山有两代控制台，鉴权方式不同，密钥也不是同一个东西。 */
export type DoubaoConsole = 'new' | 'legacy'

/** 存储键。集中放在这里，避免各处手写字符串写错。 */
export const DOUBAO_KEYS = {
  /** 用户选的控制台代次 */
  console: 'cloudAsr.doubao.console',
  /** 新版控制台的 APP Key（发 X-Api-Key） */
  consoleKey: 'cloudAsr.doubao.consoleKey',
  /** 旧版控制台的 Access Token（发 X-Api-Access-Key）——历史键，语义不变 */
  accessToken: 'cloudAsr.doubao.apiKey',
  /** 旧版控制台的 App ID（发 X-Api-App-Key）——历史键，语义不变 */
  appId: 'cloudAsr.doubao.appId',
} as const

/**
 * 判定当前该用哪代控制台。
 *
 * 存过就用存的值；没存过（老版本升级上来）看有没有 App ID —— 旧版控制台必须填 App ID，
 * 所以「有 App ID」就等于「这是个旧版用户」，行为保持不变，不需要他重新配置。
 * 全新用户没有任何历史值，默认新版：只填一个 key，接入门槛最低。
 */
export function resolveDoubaoConsole(
  saved: string | null | undefined,
  legacyAppId: string,
): DoubaoConsole {
  if (saved === 'new' || saved === 'legacy') return saved
  return legacyAppId.trim() ? 'legacy' : 'new'
}

export interface DoubaoCredentials {
  console: DoubaoConsole
  /** 新版控制台的 APP Key */
  consoleKey: string
  /** 旧版控制台的 Access Token */
  accessToken: string
  /** 旧版控制台的 App ID */
  appId: string
}

/**
 * 算出「本次生效的凭据」，写进运行时读的那两个扁平键（cloudAsr.apiKey / cloudAsr.appId）。
 *
 * **关键不变量：新版模式下 appId 必须是空串。** Rust 侧就是用「app_id 空不空」来决定
 * 发 X-Api-Key 还是发 X-Api-App-Key + X-Api-Access-Key 的。这里漏掉清空，线上就会用
 * 旧版格式发新版的密钥，拿回一个看不出原因的鉴权失败。
 *
 * 两代的密钥各自留在自己的存储键里，来回切不会丢，不用重新粘贴。
 */
export function effectiveDoubaoCredentials(
  creds: DoubaoCredentials,
): { apiKey: string; appId: string } {
  if (creds.console === 'new') {
    return { apiKey: creds.consoleKey.trim(), appId: '' }
  }
  return { apiKey: creds.accessToken.trim(), appId: creds.appId.trim() }
}

/** 密钥输入框的显示名——两代叫法不同，标错了用户会去控制台找一个不存在的东西。 */
export function doubaoKeyLabel(console: DoubaoConsole): string {
  return console === 'new' ? 'API Key' : 'Access Token'
}

/**
 * 还缺什么才能用（空串 = 齐了）。文案直接可展示，就绪指示和设置页共用同一句话。
 */
export function describeDoubaoMissing(creds: DoubaoCredentials): string {
  if (creds.console === 'new') {
    return creds.consoleKey.trim() ? '' : t('asrProvider.missingKey')
  }
  if (!creds.accessToken.trim()) return t('asrProvider.missingAccessToken')
  if (!creds.appId.trim()) return t('asrProvider.missingAppId')
  return ''
}
