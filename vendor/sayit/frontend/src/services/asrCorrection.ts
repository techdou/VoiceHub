import { t } from '@/i18n'
import * as bridge from '@/services/bridge'
import { getBackendBaseUrl } from '@/services/runtimeConfig'
import { getSetting, setSetting, type HistoryRecord } from '@/services/store'
import { normalizeForDiff } from '@/lib/asrDiff'

/**
 * ASR 纠错反馈：用户主动把「这条识别错了，正确的是这样」连同当次录音提交给服务端。
 *
 * 与「服务端静默录音」不是一回事：这里的每一次上传都由用户在历史记录里点出来，
 * 首次提交前还要过一次明确的同意确认。详见 .kiro/decisions.md。
 *
 * 走 webview 的 fetch + FormData，不走 Rust：
 * 浏览器会自带 User-Agent，天然避开"reqwest 不带 UA 被 WAF 403"那个坑（pitfalls #1）。
 */

/** 同意文案的版本。文案实质变化时 +1，老用户会被要求重新确认。 */
export const ASR_CORRECTION_CONSENT_VERSION = '1'
const CONSENT_SETTING_KEY = 'asrCorrectionConsentVersion'

/**
 * 承诺给用户的撤回窗口（天），只用于文案。改这里要同时改后端的 WITHDRAW_DAYS。
 *
 * 后端**没有**按这个天数拦撤回：承诺是"30 天内可以撤回"，那么放宽到永远可撤回
 * 永远不会违背承诺，而加一道硬期限只会多出一个"点了没反应"的失败分支。
 */
export const ASR_CORRECTION_WITHDRAW_DAYS = 30

/** 与后端 MAX_AUDIO_BYTES 一致：五分钟录音约 9.6MB，10 MiB 装得下。 */
export const MAX_CORRECTION_AUDIO_BYTES = 10 * 1024 * 1024

export type AsrCorrectionErrorCode =
  | 'audio_missing'
  | 'audio_too_large'
  | 'no_change'
  | 'rate_limited'
  | 'already_submitted'
  | 'storage_full'
  | 'network'
  /** 这台后端没有这个接口（自建服务器版本较旧，或官方后端还没更新） */
  | 'not_supported'
  | 'server'

export interface AsrCorrectionResult {
  ok: boolean
  correctionId?: string
  code?: AsrCorrectionErrorCode
  message: string
}

export async function hasAsrCorrectionConsent(): Promise<boolean> {
  const stored = await getSetting<string>(CONSENT_SETTING_KEY, '')
  return stored === ASR_CORRECTION_CONSENT_VERSION
}

export async function grantAsrCorrectionConsent(): Promise<void> {
  await setSetting(CONSENT_SETTING_KEY, ASR_CORRECTION_CONSENT_VERSION)
}

/**
 * 由历史记录 id 推出提交编号，而不是每次随机生成。
 *
 * 这样重试（上一次请求超时但服务端其实收下了）用的是同一个编号，服务端按编号幂等，
 * 不会出现两条一样的样本；用户要撤回时也能凭本地记录算出同一个编号。
 * 形状必须匹配后端的 ^[A-Za-z0-9_-]{8,64}$。
 */
export function buildCorrectionId(recordId: string): string {
  const safe = (recordId || '').replace(/[^A-Za-z0-9_-]/g, '')
  return `c-${safe}`.padEnd(8, '0').slice(0, 64)
}

// 不写返回类型注解：显式写 `Uint8Array` 会退化成 Uint8Array<ArrayBufferLike>，
// 而 BlobPart 只接受 Uint8Array<ArrayBuffer>，会在 new Blob 那行报类型错。
// 让它从 new Uint8Array(length) 推断出来正好是对的。
function base64ToBytes(base64: string) {
  const binary = atob(base64)
  const bytes = new Uint8Array(binary.length)
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i)
  return bytes
}

function messageForCode(code: AsrCorrectionErrorCode, status?: number): string {
  switch (code) {
    case 'audio_missing':
      return t('asrCorrection.errorAudioMissing')
    case 'audio_too_large':
      return t('asrCorrection.errorAudioTooLarge')
    case 'no_change':
      return t('asrCorrection.errorNoChange')
    case 'rate_limited':
      return t('asrCorrection.errorRateLimited')
    case 'already_submitted':
      return t('asrCorrection.errorAlreadySubmitted')
    case 'storage_full':
      return t('asrCorrection.errorStorageFull')
    case 'network':
      return t('asrCorrection.errorNetwork')
    case 'not_supported':
      return t('asrCorrection.errorNotSupported')
    default:
      return t('asrCorrection.errorServer', { status: status ?? 0 })
  }
}

/** 后端 error code → 我们的分类。没列到的都归 server（界面上显示状态码，便于排查）。 */
function classifyServerError(error: string): AsrCorrectionErrorCode {
  if (error === 'rate_limited') return 'rate_limited'
  if (error === 'already_submitted') return 'already_submitted'
  if (error === 'storage_full') return 'storage_full'
  if (error === 'audio_too_large') return 'audio_too_large'
  if (error === 'no_change') return 'no_change'
  return 'server'
}

export async function submitAsrCorrection(
  record: HistoryRecord,
  correctedText: string,
): Promise<AsrCorrectionResult> {
  const original = normalizeForDiff(record.asrText || '')
  const corrected = normalizeForDiff(correctedText)
  if (!original || !corrected) {
    return { ok: false, code: 'no_change', message: messageForCode('no_change') }
  }
  if (original === corrected) {
    return { ok: false, code: 'no_change', message: messageForCode('no_change') }
  }

  if (!record.audioFilePath) {
    return { ok: false, code: 'audio_missing', message: messageForCode('audio_missing') }
  }
  // 保留期到了之后 Rust 只删文件不清路径，所以这里必须按"读不到就是没有"处理
  const base64 = await bridge.readAudioFile(record.audioFilePath).catch(() => null)
  if (!base64) {
    return { ok: false, code: 'audio_missing', message: messageForCode('audio_missing') }
  }

  const bytes = base64ToBytes(base64)
  if (bytes.byteLength > MAX_CORRECTION_AUDIO_BYTES) {
    // 本地先拦一次，别让用户传完 10MB 才看到失败
    return { ok: false, code: 'audio_too_large', message: messageForCode('audio_too_large') }
  }

  const clientInfo = await bridge.getClientRuntimeInfo().catch(() => null)
  const correctionId = buildCorrectionId(record.id)

  const form = new FormData()
  form.append('audio', new Blob([bytes], { type: 'audio/wav' }), 'recording.wav')
  form.append('correction_id', correctionId)
  form.append('machine_id', clientInfo?.deviceId || 'unknown')
  form.append('original_asr_text', original)
  form.append('corrected_text', corrected)
  form.append('work_mode', record.workMode || 'server')
  form.append('app_version', clientInfo?.clientVersion || '')
  form.append('client_record_id', record.id)
  form.append('asr_provider', record.asrProvider || '')
  // 热词是 ASR 错误最常见的成因之一，没有它样本无法判断成因（见 pitfalls 的热词幻觉）
  form.append('hotwords', JSON.stringify(record.autoAppliedHotwords || []))
  form.append('consent_version', ASR_CORRECTION_CONSENT_VERSION)

  let response: Response
  try {
    response = await fetch(`${getBackendBaseUrl()}/api/asr-corrections`, { method: 'POST', body: form })
  } catch {
    return { ok: false, code: 'network', message: messageForCode('network') }
  }

  let body: Record<string, unknown> = {}
  try {
    body = (await response.json()) as Record<string, unknown>
  } catch {
    body = {}
  }

  if (response.ok) {
    return {
      ok: true,
      correctionId: String(body.correction_id || correctionId),
      message: t('asrCorrection.submitted'),
    }
  }

  // 「这条音频已经提交过」也算成功：本地标记成已提交，用户不会反复看到入口
  const errorCode = String(body.error || '')
  if (response.status === 409 && errorCode === 'already_submitted') {
    return {
      ok: true,
      correctionId: String(body.correction_id || correctionId),
      message: t('asrCorrection.alreadySubmitted'),
    }
  }

  // 后端没有这个接口时不要报成"提交失败(405)"。两种真实情况都会走到这里：
  // 用户把后端指到自建服务器而那边版本较旧；官方后端还没更新到带这个接口的版本。
  // （SayIt 后端把静态站点挂在 "/" 上，所以路径不存在时拿到的是 404，方法不匹配是 405，
  //   不是 FastAPI 的标准 405 —— 见 pitfalls #23。）
  if (response.status === 404 || response.status === 405) {
    return { ok: false, code: 'not_supported', message: messageForCode('not_supported') }
  }

  const code = classifyServerError(errorCode)
  return { ok: false, code, message: messageForCode(code, response.status) }
}

/** 撤回：服务端会删掉音频并清掉文本。失败返回 false，调用方保持"已提交"状态不变。 */
export async function withdrawAsrCorrection(correctionId: string): Promise<boolean> {
  const clientInfo = await bridge.getClientRuntimeInfo().catch(() => null)
  try {
    const response = await fetch(
      `${getBackendBaseUrl()}/api/asr-corrections/${encodeURIComponent(correctionId)}/withdraw`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ machine_id: clientInfo?.deviceId || 'unknown' }),
      },
    )
    // 404 = 服务端已经没有这条了（保留期到了或已清理），对用户而言目的已达成
    return response.ok || response.status === 404
  } catch {
    return false
  }
}
