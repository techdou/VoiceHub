// 转写 Provider 抽象层类型定义
// 所有工作模式（服务器 / 云 API / 本地）共享此接口

import type { ActiveAppContext, TextContext } from '../../types/appContext'
import type { ClientRuntimeInfo } from '../../types/appApi'

export type WorkMode = 'server' | 'cloud_api' | 'local'

export type ProviderState = 'disconnected' | 'connecting' | 'connected' | 'error'

export type AiExecutionSource = 'server' | 'custom' | 'none'
export type AiExecutionStatus = 'applied' | 'skipped' | 'unavailable' | 'failed'

export interface ASRResult {
  text: string
  asrMs: number
  durationSec: number
  /** 空 ASR 会先于 final 触发专用收尾，执行元数据必须随它一起落库。 */
  aiSource?: AiExecutionSource
  aiStatus?: AiExecutionStatus
}

export interface FinalResult {
  asrText: string
  llmText: string
  asrMs: number
  llmMs: number
  durationSec: number
  asrEngine?: string
  asrModel?: string
  /** True only when the AI actually received and processed this run's editor context. */
  contextApplied?: boolean
  /** Explicit execution metadata; text equality cannot prove whether AI ran. */
  aiSource?: AiExecutionSource
  aiStatus?: AiExecutionStatus
  aiProvider?: string
  aiModel?: string
}

export interface TranscriptionCallbacks {
  onStateChange?: (state: ProviderState) => void
  onReady?: (info: { connectionId?: string; asr: boolean; llm: boolean }) => void
  /** 流式识别过程中的中间结果（实时上屏用），text 为到目前为止的累计文本 */
  onPartialASR?: (text: string) => void
  onASR?: (result: ASRResult) => void
  onFinal?: (result: FinalResult) => void
  onDone?: () => void
  onError?: (msg: string) => void
}

export interface StartOptions {
  /** 当前录音代次；取消或开始下一次后，旧代次的异步结果必须全部丢弃。 */
  runId: number
  systemPrompt?: string
  disableAi?: boolean
  /** 录音未达到该时长时只做识别，不调用 AI。0 / undefined = 不设门槛。 */
  aiMinDurationSec?: number
  clientMeta?: ClientRuntimeInfo | null
  appContext?: ActiveAppContext | null
  /** Bounded editor text captured at recording start. Never persisted in history/logs. */
  textContext?: TextContext | null
  source?: 'live' | 'history_reprocess'
  hotwords?: string[]
  language?: string
  /** 是否开启流式实时显示：识别过程中把中间结果实时推给悬浮窗 */
  streamingDisplay?: boolean
}

export interface StopOptions {
  pttHoldMs?: number
  /**
   * 松键时才知道的「这次别做 AI 整理」。目前只有短语音门槛会用到：录音时长要等录完
   * 才知道，而服务器模式的 AI 在服务端紧跟 ASR 执行，start 时来不及决定。
   * 只能追加跳过理由，不能反过来把 AI 打开。
   */
  disableAi?: boolean
  audioStats?: {
    avgRms: number
    peakRms: number
    peakAmplitude: number
    silenceRatio: number
    totalFrames: number
  }
}

/**
 * 转写 Provider 接口。
 * RecorderOrchestrator 通过此接口与具体的转写实现交互，
 * 不再直接依赖 WebSocket 或任何特定的后端协议。
 */
export interface TranscriptionProvider {
  readonly mode: WorkMode

  /** 建立连接 / 初始化 */
  connect(callbacks: TranscriptionCallbacks): Promise<void>

  /** 开始一次转写会话 */
  start(opts: StartOptions): boolean

  /** 立即取消当前会话；允许底层任务自然结束，但之后不得再上抛任何结果。 */
  cancel(): void

  /** 发送音频数据（流式，PCM Int16 ArrayBuffer） */
  sendAudio(buffer: ArrayBuffer): void

  /** 结束录音，触发处理 */
  stop(opts?: StopOptions): boolean

  /** 断开连接 / 释放资源 */
  disconnect(): void

  /** 是否已就绪可以开始转写 */
  isReady(): boolean
}
