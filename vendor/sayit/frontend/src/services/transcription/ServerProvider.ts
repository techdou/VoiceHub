// 服务器模式 Provider：服务器负责 ASR，AI 可由服务器或客户端自定义供应商完成。

import { getSetting } from '../store'
import { addRuntimeEvent } from '../debugLog'
import * as ws from '../websocket'
import { polishWithClientAi } from './clientAiPolish'
import {
  getRuntimeServerAiSource,
  loadServerAiSource,
  type ServerAiSource,
} from './serverAiSource'
import type {
  FinalResult,
  TranscriptionProvider,
  TranscriptionCallbacks,
  StartOptions,
  StopOptions,
} from './types'

export class ServerProvider implements TranscriptionProvider {
  readonly mode = 'server' as const

  private callbacks: TranscriptionCallbacks = {}
  private activeRunId = 0
  private activeStartOpts: Readonly<StartOptions> | undefined
  private aiSource: ServerAiSource = 'managed'
  private customAiReady = false
  private customFinalPending = false
  private serverDonePending = false

  async connect(callbacks: TranscriptionCallbacks): Promise<void> {
    this.callbacks = callbacks
    const [storedSource, customProvider, customUrl, customKey, customModel] = await Promise.all([
      loadServerAiSource(),
      getSetting('cloudAi.provider', '') as Promise<string>,
      getSetting('cloudAi.apiUrl', '') as Promise<string>,
      getSetting('cloudAi.apiKey', '') as Promise<string>,
      getSetting('cloudAi.model', '') as Promise<string>,
    ])
    this.aiSource = storedSource === 'custom' ? 'custom' : 'managed'
    this.customAiReady = Boolean(
      customUrl.trim()
      && customModel.trim()
      && (customKey.trim() || customProvider === 'ollama'),
    )

    await ws.connect({
      onStateChange: (state) => {
        callbacks.onStateChange?.(state)
      },
      onReady: (data) => {
        callbacks.onReady?.({
          connectionId: data.connectionId,
          asr: data.asr,
          // 自定义 AI 在本机执行；服务端 llm=false 不应把整条模式标成不可用。
          llm: this.aiSource === 'custom' ? this.customAiReady : data.llm,
        })
      },
      onASR: (result) => {
        if (this.activeRunId === 0) return
        const isEmpty = !result.text.trim()
        callbacks.onASR?.({
          text: result.text,
          asrMs: result.asrMs,
          durationSec: result.durationSec,
          // 空识别不会执行任何 AI；它会在录音器的专用空结果分支先于 final 落库。
          aiSource: isEmpty ? 'none' : undefined,
          aiStatus: isEmpty ? 'skipped' : undefined,
        })
      },
      onFinal: (result) => {
        const runId = this.activeRunId
        if (runId === 0) return
        if (this.aiSource === 'custom') {
          this.customFinalPending = true
          void this.handleCustomFinal(runId, result)
          return
        }

        const aiDisabled = this.activeStartOpts?.disableAi ?? false
        callbacks.onFinal?.({
          asrText: result.asrText,
          llmText: result.llmText,
          asrMs: result.asrMs,
          llmMs: result.llmMs,
          durationSec: result.durationSec,
          asrEngine: result.asrEngine,
          asrModel: result.asrModel,
          contextApplied: result.contextApplied,
          aiSource: aiDisabled ? 'none' : 'server',
          aiStatus: aiDisabled ? 'skipped' : result.llmMs > 0 ? 'applied' : 'unavailable',
          aiProvider: aiDisabled ? undefined : 'server',
        })
      },
      onDone: () => {
        const runId = this.activeRunId
        if (runId === 0) return
        if (this.customFinalPending) {
          // 服务端 final 后会立刻发 done；客户端 AI 尚未完成时必须暂存，避免录音器先收尾。
          this.serverDonePending = true
          return
        }
        this.finishRun(runId)
      },
      onError: (msg) => {
        const runId = this.activeRunId
        if (runId === 0) return
        callbacks.onError?.(msg)
        if (this.activeRunId === runId) this.resetRun()
      },
    })
  }

  start(opts: StartOptions): boolean {
    // 设置页可在不切换工作模式的情况下改变来源；每次 start 都同步取运行时真值。
    this.aiSource = getRuntimeServerAiSource()
    this.activeRunId = opts.runId
    this.activeStartOpts = {
      ...opts,
      hotwords: opts.hotwords ? [...opts.hotwords] : undefined,
      textContext: opts.textContext ? { ...opts.textContext } : undefined,
    }
    this.customFinalPending = false
    this.serverDonePending = false

    const wireOptions = this.aiSource === 'custom'
      ? {
        ...opts,
        // 自定义 AI 由本机调用：服务端只做 ASR，也不需要收到 Prompt 或编辑器正文。
        disableAi: true,
        systemPrompt: undefined,
        textContext: undefined,
      }
      : opts
    const started = ws.sendStart(wireOptions)
    if (!started) this.resetRun()
    return started
  }

  cancel(): void {
    this.resetRun()
    ws.disconnect()
  }

  sendAudio(buffer: ArrayBuffer): void {
    ws.sendAudio(buffer)
  }

  stop(opts?: StopOptions): boolean {
    if (opts?.disableAi && this.activeStartOpts) {
      this.activeStartOpts = { ...this.activeStartOpts, disableAi: true }
    }
    return ws.sendStop(this.aiSource === 'custom' ? { ...opts, disableAi: true } : opts)
  }

  disconnect(): void {
    this.resetRun()
    ws.disconnect()
  }

  isReady(): boolean {
    return ws.isConnected()
  }

  private async handleCustomFinal(runId: number, result: FinalResult): Promise<void> {
    const startOptions = this.activeStartOpts
    const polished = await polishWithClientAi({
      asrText: result.asrText,
      durationSec: result.durationSec,
      startOptions,
      logSource: 'server',
      isCurrent: () => this.activeRunId === runId,
    })
    if (!polished || this.activeRunId !== runId) return

    this.callbacks.onFinal?.({
      ...result,
      ...polished,
    })
    this.customFinalPending = false

    if (this.serverDonePending) {
      this.finishRun(runId)
    }
  }

  private finishRun(runId: number): void {
    if (this.activeRunId !== runId) return
    this.callbacks.onDone?.()
    if (this.activeRunId === runId) this.resetRun()
  }

  private resetRun(): void {
    if (this.activeRunId !== 0 && this.customFinalPending) {
      addRuntimeEvent('info', 'server', 'Discarded pending custom AI result for stale run', {
        runId: this.activeRunId,
      })
    }
    this.activeRunId = 0
    this.activeStartOpts = undefined
    this.customFinalPending = false
    this.serverDonePending = false
  }
}
