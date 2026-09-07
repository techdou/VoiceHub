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
