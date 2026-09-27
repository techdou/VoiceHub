/**
 * RecorderOrchestrator 纯辅助函数
 * 不依赖 this 状态，可独立测试
 */

import type { ActiveAppContext } from '@/types/appContext'

/** 简化 AppContext 用于日志输出 */
export function summarizeAppContext(context: ActiveAppContext | null) {
  if (!context) return null
  return {
    processName: context.processName,
    exePath: context.exePath,
    windowTitle: context.windowTitle,
    windowClass: context.windowClass,
    focusClass: context.focusClass,
    controlType: context.controlType,
    focusedName: context.focusedName,
  }
}

/** 从 AppContext 中提取用于统计的 appId */
export function buildStatsAppId(
  appContext: ActiveAppContext | null,
  promptAppId?: string,
): string {
  const processName = String(appContext?.processName || '').trim()
  if (processName) return processName

  const exePath = String(appContext?.exePath || '').trim()
  if (exePath) {
    const segments = exePath.split(/[\\/]/).filter(Boolean)
    const lastSegment = segments[segments.length - 1]
    if (lastSegment) return lastSegment
  }

  return String(promptAppId || '').trim() || 'unknown'
}

/** 录音时的麦克风音量分级 */
export type MicLevel = 'muted' | 'low' | 'voiced'

/** 只有峰值严格为 0（PCM 整帧全 0）才算「没有信号」。
 * 再小的非零采样也是真实存在的输入，只能归为声音偏低，不能提示「未检测到声音」。 */
export const MIC_NO_SIGNAL_PEAK_THRESHOLD = 0
/** 有非零信号但 RMS（0..1）低于此值时，视为「声音偏低」（离得远 / 说太小声）。 */
export const MIC_LOW_RMS_THRESHOLD = 0.008

/**
 * 按本帧的 RMS 与峰值分级：
 *  - muted ：整帧 PCM 全 0（极可能麦克风被静音 / 选错设备 / 未授权）
 *  - low   ：有波动但整体偏低（距离远 / 声音小）
 *  - voiced：正常音量
 * 两个入参都应归一化到 0..1（原始 int16 除以 32768）。
 */
export function classifyMicLevel(rms: number, framePeak: number): MicLevel {
  if (framePeak <= MIC_NO_SIGNAL_PEAK_THRESHOLD) return 'muted'
  if (rms < MIC_LOW_RMS_THRESHOLD) return 'low'
  return 'voiced'
}

/** 判「整段录音确实没有可用语音」时，静音帧至少要占到这个比例。 */
export const SILENCE_EVIDENCE_MIN_RATIO = 0.995

/**
 * 这段录音有没有「确实没有可用语音」的实测证据。
 *
 * 为什么需要证据：「未检测到有效声音」是全软件最容易被误读的一句话，它会把用户直接
 * 引去查麦克风。而同一句话背后的真实成因可能是额度耗尽、服务端提前断开、热词回显
 * 被判定清空（见 pitfalls 15）。没有证据时就该说「没有取得识别结果」，别替用户下结论。
 *
 * ⚠️ 两个条件必须**同时**成立，绝不能取或。高静音比例单独什么都证明不了 —— 说一句话
 * 再沉默一分钟，静音帧就能占到 99.6%，而峰值 0.4（清清楚楚有人在说话）。取或的写法
 * 会把这种录音判成"没声音"，真正的失败原因反而被藏起来。
 *
 * 入参都是采集侧的统计量，与 ASR/网络无关；峰值归一化到 0..1。
 */
export function hasSilenceEvidence(stats: {
  totalFrames: number
  silentFrames: number
  peakAmplitude: number
  silenceRmsThreshold: number
}): boolean {
  if (stats.totalFrames <= 0) return false
  const silenceRatio = stats.silentFrames / stats.totalFrames
  return stats.peakAmplitude < stats.silenceRmsThreshold
    && silenceRatio > SILENCE_EVIDENCE_MIN_RATIO
}

/** 系统报端点静音后，需要累计这么多个全 0 采样（~300ms @16kHz）才确认麦克风真的没在收音。
 *  取 300ms：快到仍像「即时反馈」，又足以跨过采集刚打开时可能出现的空帧。 */
export const OS_MIC_MUTE_CONFIRM_SAMPLES = 4800

/** 对「系统报麦克风被静音」这条线索的裁决结果。
 *  - wait     ：还在攒证据，silentSamples 是更新后的累计值
 *  - confirmed：系统标志与音频信号一致，可以报「麦克风已被静音」
 *  - dismissed：音频在正常流动，系统标志不作数，本次录音丢弃这条线索 */
export type OsMicMuteDecision =
  | { verdict: 'wait'; silentSamples: number }
  | { verdict: 'confirmed' }
  | { verdict: 'dismissed' }

/**
 * 用真实音频信号裁决系统的麦克风静音标志。
 *
 * 为什么需要裁决而不能直接采信系统：`IAudioEndpointVolume::GetMute` 反映的是端点上的静音
 * **设置**，不等于采集流真被切断。实测 Plantronics Blackwire 5220 USB 耳麦会停在
 * GetMute=true，而 getUserMedia 照常收到正常音量的音频——直接采信就会每次按热键都先弹一次
 * 红色高警「麦克风已被静音」，说话 0.5s 后又自己消失。
 *
 * 于是：只有全 0 的 PCM 累计到阈值（两个证据一致）才确认；一旦出现**任何**非零采样，就说明
 * 这块设备的 mute 标志不作数，立刻永久丢弃线索——不是清零重来，否则说话的自然停顿会把它攒回来。
 */
export function judgeOsMicMute(
  silentSamples: number,
  level: MicLevel,
  sampleCount: number,
): OsMicMuteDecision {
  if (level !== 'muted') return { verdict: 'dismissed' }
  const total = silentSamples + sampleCount
  if (total >= OS_MIC_MUTE_CONFIRM_SAMPLES) return { verdict: 'confirmed' }
  return { verdict: 'wait', silentSamples: total }
}

/** 判断 PTT 设置中是否包含修饰键（旧单键与物理组合格式都支持） */
export function isModifierPTTSetting(pttSetting?: string): boolean {
  if (!pttSetting) return false
  return pttSetting.split('+').some((code) => (
    code.startsWith('Alt')
    || code.startsWith('Control')
    || code.startsWith('Shift')
    || code.startsWith('Meta')
  ))
}

/**
 * 模拟 Ctrl+V 耗时超过这个值，就不再当作插入成功。
 *
 * 为什么：SendInput 只是把按键塞进队列，目标程序卡住时，它会在我们 400ms 后恢复剪贴板
 * **之后**才处理 Ctrl+V —— 于是什么都没粘上（或粘进了用户原来的剪贴板），而这一侧照样
 * 报成功、不弹卡片。慢本身就说明目标或系统在卡：耗时主要花在把目标切到前台那一步，
 * 它要等目标线程回应。
 * 门槛取自真实日志：数百次 send_input 最慢 631ms，出事那次 2088ms。
 */
export const SLOW_SEND_INPUT_PASTE_MS = 1200

/**
 * 这次"成功"的插入是否拿不准、需要弹兜底卡片。只针对 send_input：WM_PASTE 与控制台粘贴
 * 会核实结果或同步等目标处理完，慢不代表没粘上。
 */
export function isUnconfirmedPaste(strategy: string | undefined, pasteExecMs: number): boolean {
  return strategy === 'send_input' && pasteExecMs >= SLOW_SEND_INPUT_PASTE_MS
}

const PROCESSING_TIMEOUT_BASE_MS = 15_000
const PROCESSING_TIMEOUT_PER_AUDIO_SEC_MS = 500
const PROCESSING_TIMEOUT_MAX_EXTRA_MS = 30_000

/** 根据音频时长和工作模式计算处理超时时间 */
export function computeProcessingTimeoutMs(
  audioDurationSec: number,
  providerMode: string,
): number {
  const safeAudioSec = Number.isFinite(audioDurationSec) ? Math.max(0, audioDurationSec) : 0
  const extraMs = Math.min(
    PROCESSING_TIMEOUT_MAX_EXTRA_MS,
    Math.ceil(safeAudioSec * PROCESSING_TIMEOUT_PER_AUDIO_SEC_MS),
  )
  let timeout = PROCESSING_TIMEOUT_BASE_MS + extraMs

  if (providerMode !== 'server') {
    timeout = Math.max(timeout, 30000)
  }
  if (providerMode === 'cloud_api') {
    const cloudTimeout = 30000 + Math.ceil(safeAudioSec * 500)
    timeout = Math.min(Math.max(timeout, cloudTimeout), 90000)
  }
  return timeout
}
