/**
 * 缓冲式 Provider 基类
 * CloudAPIProvider 和 LocalProvider 共享的 PCM 缓冲、合并、生命周期逻辑。
 * 子类只需实现 onConnect() 和 processAudio()。
 */

import { uint8ArrayToBase64 } from '@/lib/encoding'
import { addRuntimeEvent } from '../debugLog'
import type {
  TranscriptionProvider,
  TranscriptionCallbacks,
  StartOptions,
  StopOptions,
  WorkMode,
} from './types'

export abstract class BufferedProvider implements TranscriptionProvider {
  abstract readonly mode: WorkMode

  protected callbacks: TranscriptionCallbacks = {}
  protected pcmBuffers: ArrayBuffer[] = []
  protected sessionActive = false
  protected startOpts: StartOptions | undefined
  protected ready = false
  protected activeRunId = 0

  /**
   * 子类可覆盖，用于连接时的额外初始化（如预加载模型）。
   *
   * **返回 false 表示"连上了但用不了"**，此时 isReady() 会是 false，
   * RecorderOrchestrator 那道前置拦截就会挡住录音并给出提示。
   * 返回 void / true 视为就绪（CloudAPIProvider 等不关心的子类无需改动）。
   */
  protected async onConnect(_callbacks: TranscriptionCallbacks): Promise<void | boolean> { }

  async connect(callbacks: TranscriptionCallbacks): Promise<void> {
    this.callbacks = callbacks
    this.ready = true
    callbacks.onStateChange?.('connected')
    // 以前这里只是 await，子类无从否决 —— 于是本地模式把模型全删了 isReady() 仍为 true，
    // 按快捷键照样开录、录完才在识别阶段报错（而设置页写着"按下快捷键不会有反应"）。
    const usable = await this.onConnect(callbacks)
    if (usable === false) {
      this.ready = false
      addRuntimeEvent('warn', this.mode, 'Provider connected but is unavailable; marked not ready')
    }
  }

  start(opts: StartOptions): boolean {
    if (!this.ready) {
      addRuntimeEvent('error', this.mode, 'Start failed: provider is not ready')
      return false
    }
    this.pcmBuffers = []
    this.sessionActive = true
    this.startOpts = opts
    this.activeRunId = opts.runId
    return true
  }

  cancel(): void {
    this.activeRunId = 0
    this.sessionActive = false
    this.pcmBuffers = []
    this.startOpts = undefined
  }

  sendAudio(buffer: ArrayBuffer): void {
    if (!this.sessionActive) return
    this.pcmBuffers.push(buffer.slice(0))
  }

  stop(_opts?: StopOptions): boolean {
    if (!this.sessionActive) return false
    const runId = this.activeRunId
    this.sessionActive = false
    void this.runProcessAudio(runId)
    return true
  }

  disconnect(): void {
    this.cancel()
    this.ready = false
    this.callbacks.onStateChange?.('disconnected')
  }

  isReady(): boolean {
    return this.ready
  }

  // ─── 子类实现 ───

  /**
   * 处理合并后的音频数据。
   * @param audioB64 base64 编码的 PCM 数据
   * @param durationSec 音频时长（秒）
   */
  protected abstract processAudio(audioB64: string, durationSec: number, runId: number): Promise<void>

  protected isRunCurrent(runId: number): boolean {
    return runId !== 0 && this.activeRunId === runId
  }

  // ─── 内部逻辑 ───

  private async runProcessAudio(runId: number): Promise<void> {
    try {
      if (!this.isRunCurrent(runId)) return
      const totalBytes = this.pcmBuffers.reduce((sum, buf) => sum + buf.byteLength, 0)
      if (totalBytes === 0) {
        if (this.isRunCurrent(runId)) {
          this.callbacks.onDone?.()
          if (this.isRunCurrent(runId)) this.activeRunId = 0
        }
        return
      }

      const merged = new Uint8Array(totalBytes)
      let offset = 0
      for (const buf of this.pcmBuffers) {
        merged.set(new Uint8Array(buf), offset)
        offset += buf.byteLength
      }
      this.pcmBuffers = []

      const durationSec = (totalBytes / 2) / 16000
      if (durationSec < 0.3) {
      addRuntimeEvent('info', this.mode, 'Audio too short; skipped processing', { durationSec })
        if (this.isRunCurrent(runId)) {
          this.callbacks.onDone?.()
          if (this.isRunCurrent(runId)) this.activeRunId = 0
        }
        return
      }

      const audioB64 = uint8ArrayToBase64(merged)
      await this.processAudio(audioB64, durationSec, runId)
      if (this.isRunCurrent(runId)) this.activeRunId = 0
    } catch (err) {
      if (!this.isRunCurrent(runId)) return
      addRuntimeEvent('error', this.mode, 'Processing failed', { error: String(err) })
      this.callbacks.onError?.(String(err))
      this.callbacks.onDone?.()
      if (this.isRunCurrent(runId)) this.activeRunId = 0
    }
  }
}
