// 云 API 模式 Provider
// 豆包 ASR：边录边发（实时流式）
// 其他 ASR：录完再发（BufferedProvider）

import { isQwenOmniProvider, isStreamingDisplayReady, resolveQwenOmniModel } from '@/lib/asrModels'
import { uint8ArrayToBase64 } from '@/lib/encoding'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getSetting } from '../store'
import { restoreHotwordSpacing } from '../textPostProcess'
import { addRuntimeEvent } from '../debugLog'
import { polishWithClientAi } from './clientAiPolish'
import type {
  TranscriptionProvider,
  TranscriptionCallbacks,
  StartOptions,
  StopOptions,
  WorkMode,
} from './types'

interface AsrProviderConfig {
  provider: string
  api_key: string
  app_id: string
  extra?: Record<string, unknown>
}

interface AsrResult { text: string; elapsed_ms: number }

/**
 * 千问两个流式模型各有一组原生命令。
 *
 * 它们是**两套 WebSocket 协议**，不是同一个接口的两个模型名：
 *   · qwen_realtime      → OpenAI-Realtime 风格（/api-ws/v1/realtime）
 *   · qwen_audio_stream  → DashScope duplex run-task（/api-ws/v1/inference）
 * Rust 侧因此各有一份实现（asr_qwen_realtime.rs / asr_qwen_audio_stream.rs），
 * 这里要记住本次会话用的是哪一组，收尾时才不会去 finish 另一条没开的会话。
 */
const QWEN_STREAM_COMMANDS = {
  qwen_realtime: {
    open: 'qwen_stream_open',
    send: 'qwen_stream_send',
    finish: 'qwen_stream_finish',
    close: 'qwen_stream_close',
  },
  qwen_audio_stream: {
    open: 'qwen_audio_stream_open',
    send: 'qwen_audio_stream_send',
    finish: 'qwen_audio_stream_finish',
    close: 'qwen_audio_stream_close',
  },
} as const

type QwenStreamProvider = keyof typeof QWEN_STREAM_COMMANDS
type QwenStreamCommands = typeof QWEN_STREAM_COMMANDS[QwenStreamProvider]
type NativeOpenCommand = 'doubao_stream_open' | QwenStreamCommands['open']
type NativeFinishCommand = 'doubao_stream_finish' | QwenStreamCommands['finish']

function isQwenStreamProvider(provider: string): provider is QwenStreamProvider {
  return provider === 'qwen_realtime' || provider === 'qwen_audio_stream'
}

export class CloudAPIProvider implements TranscriptionProvider {
  readonly mode: WorkMode = 'cloud_api'

  private callbacks: TranscriptionCallbacks = {}
  private pcmBuffers: ArrayBuffer[] = []
  private sessionActive = false
  private activeRunId = 0
  private activeStartOpts: Readonly<StartOptions> | undefined
  private ready = false

  // 豆包/千问流式状态
  private isDoubaoStream = false
  private isQwenStream = false
  private doubaoStreamReady = false
  private qwenStreamReady = false
  /** 本次会话用的千问命令组；没走千问流式时为 null */
  private qwenCommands: QwenStreamCommands | null = null
  private streamStartTime = 0
  private pendingChunks: ArrayBuffer[] = []
  private flushTimer: ReturnType<typeof setInterval> | null = null

  // 流式实时显示：中间结果事件的取消监听函数
  private partialUnlisten: (() => void) | null = null

  // 流式发送串行化：所有音频包经此链路顺序发送，保证收尾负包一定排在最后，
  // 避免「音频包在负包之后到达」导致服务端报 last packet has been received already。
  private sendLock: Promise<void> = Promise.resolve()
  private streamFinishing = false

  // 原生流生命周期串行化：open / finish / close 共用 Provider 级队列。
  // cancel 会先把 close 排入旧 open 之后；下一代 open 因而只能在旧 close 完成后执行。
  private nativeLifecycleQueue: Promise<void> = Promise.resolve()

  async connect(callbacks: TranscriptionCallbacks): Promise<void> {
    this.callbacks = callbacks
    this.ready = true
    callbacks.onStateChange?.('connected')
    callbacks.onReady?.({ asr: true, llm: true })
  }

  start(opts: StartOptions): boolean {
    if (!this.ready) {
      addRuntimeEvent('error', 'cloud_api', 'Start failed: provider is not ready')
      return false
    }

    // 新会话不得复用旧会话的任何逻辑状态；旧 close 会先进入生命周期队列。
    if (this.activeRunId !== 0) this.cancel()

    const runOpts = this.snapshotStartOptions(opts)
    this.pcmBuffers = []
    this.sessionActive = true
    this.activeRunId = runOpts.runId
    this.activeStartOpts = runOpts
    this.isDoubaoStream = false
    this.isQwenStream = false
    this.doubaoStreamReady = false
    this.qwenStreamReady = false
    this.qwenCommands = null
    this.streamStartTime = performance.now()
    this.pendingChunks = []
    this.streamFinishing = false
    this.sendLock = Promise.resolve()

    // 异步判断供应商并建连；所有异步步骤只使用本次 run 的 StartOptions 快照。
    void this.tryStartRealtimeStream(runOpts.runId, runOpts)

    return true
  }

  private snapshotStartOptions(opts: StartOptions): Readonly<StartOptions> {
    return {
      ...opts,
      hotwords: opts.hotwords ? [...opts.hotwords] : undefined,
      textContext: opts.textContext ? { ...opts.textContext } : undefined,
    }
  }

  private isRunCurrent(runId: number): boolean {
    return runId !== 0 && this.activeRunId === runId
  }

  private completeRun(runId: number): void {
    if (!this.isRunCurrent(runId)) return

    // 先让本次 run 失效，后续迟到结果一律丢弃；close 在队列中等待旧发送链排空。
    this.activeRunId = 0
    this.activeStartOpts = undefined
    const sendLockToDrain = this.sendLock
    this.sessionActive = false
    this.pcmBuffers = []
    this.pendingChunks = []
    this.streamFinishing = true
    this.isDoubaoStream = false
    this.isQwenStream = false
    this.doubaoStreamReady = false
    this.qwenStreamReady = false
    this.qwenCommands = null
    if (this.flushTimer) { clearInterval(this.flushTimer); this.flushTimer = null }
    this.teardownPartials()
    void this.queueNativeClose(sendLockToDrain)
  }

  private enqueueNativeLifecycle<T>(operation: () => Promise<T>): Promise<T> {
    const result = this.nativeLifecycleQueue.then(operation)
    this.nativeLifecycleQueue = result.then(() => undefined, () => undefined)
    return result
  }

  private queueNativeClose(sendLockToDrain: Promise<void>): Promise<void> {
    return this.enqueueNativeLifecycle(async () => {
      await sendLockToDrain.catch(() => { })
      // 逐个关：每个 close 在没有会话时都是无操作，所以不需要先判断开了哪一个。
      await invoke('doubao_stream_close').catch(() => { })
      await invoke('qwen_stream_close').catch(() => { })
      await invoke('qwen_audio_stream_close').catch(() => { })
    })
  }

  private invokeNativeOpen(
    runId: number,
    command: NativeOpenCommand,
    args: Record<string, unknown>,
  ): Promise<boolean> {
    return this.enqueueNativeLifecycle(async () => {
      if (!this.isRunCurrent(runId)) return false
      try {
        await invoke(command, args)
      } catch (err) {
        if (!this.isRunCurrent(runId)) return false
        throw err
      }
      if (!this.isRunCurrent(runId)) return false
      return true
    })
  }

  private invokeNativeFinish(
    runId: number,
    command: NativeFinishCommand,
  ): Promise<string | undefined> {
    return this.enqueueNativeLifecycle(async () => {
      if (!this.isRunCurrent(runId)) return undefined
      try {
        const text = await invoke<string>(command)
        if (!this.isRunCurrent(runId)) return undefined
        return text
      } catch (err) {
        if (!this.isRunCurrent(runId)) return undefined
        throw err
      }
    })
  }

  /** 订阅 Rust 上抛的流式中间识别结果，实时转发给上层用于悬浮窗上屏 */
  private async subscribePartials(runId: number): Promise<void> {
    if (this.partialUnlisten) return
    let partialCount = 0
    const unlisten = await listen<{ text?: string }>('asr-partial', (event) => {
      if (!this.isRunCurrent(runId)) return
      if (!this.sessionActive && this.pcmBuffers.length === 0) return
      const text = event.payload?.text ?? ''
      partialCount++
      // 只记录第一条，避免刷屏；证明前端确实收到了 Rust 上抛的中间结果
      if (partialCount === 1) {
        addRuntimeEvent('info', 'cloud_api', 'First streaming partial received', { textLen: text.length })
      }
      this.callbacks.onPartialASR?.(text)
    })
    if (!this.isRunCurrent(runId) || this.partialUnlisten) {
      unlisten()
      return
    }
    this.partialUnlisten = unlisten
    addRuntimeEvent('info', 'cloud_api', 'Subscribed to asr-partial events')
  }

  private teardownPartials(): void {
    if (this.partialUnlisten) {
      this.partialUnlisten()
      this.partialUnlisten = null
    }
  }

  sendAudio(buffer: ArrayBuffer): void {
    if (!this.sessionActive) return

    // 始终缓存一份（用于非豆包场景 + 音频保存）
    this.pcmBuffers.push(buffer.slice(0))

    // 豆包/千问流式：攒到 pendingChunks，由定时器批量发送。
    // 收尾阶段不再接收新音频，确保负包之后不会再有音频包。
    if ((this.isDoubaoStream || this.isQwenStream) && !this.streamFinishing) {
      this.pendingChunks.push(buffer.slice(0))
    }
  }

  stop(_opts?: StopOptions): boolean {
    if (!this.sessionActive) return false
    const runId = this.activeRunId
    const runOpts = this.activeStartOpts
    if (!runOpts || runOpts.runId !== runId) return false
    this.sessionActive = false
    void this.runProcess(runId, runOpts)
    return true
  }

  cancel(): void {
    // 必须先使 run 失效，再清理前端状态；close 最后入队且等待此刻的旧发送链。
    this.activeRunId = 0
    this.activeStartOpts = undefined
    const sendLockToDrain = this.sendLock
    this.sessionActive = false
    this.pcmBuffers = []
    this.pendingChunks = []
    this.streamFinishing = true
    this.isDoubaoStream = false
    this.isQwenStream = false
    this.doubaoStreamReady = false
    this.qwenStreamReady = false
    this.qwenCommands = null
    this.teardownPartials()
    if (this.flushTimer) { clearInterval(this.flushTimer); this.flushTimer = null }
    void this.queueNativeClose(sendLockToDrain)
  }

  disconnect(): void {
    this.cancel()
    this.ready = false
    this.callbacks.onStateChange?.('disconnected')
  }

  isReady(): boolean {
    return this.ready
  }


  // ── 豆包流式建连 ──

  private async tryStartRealtimeStream(
    runId: number,
    startOpts: Readonly<StartOptions>,
  ): Promise<void> {
    try {
      const asrProvider = await getSetting('cloudAsr.provider', 'doubao') as string
      const qwenWorkspaceId = await getSetting('cloudAsr.qwen.workspaceId', '') as string
      if (!this.isRunCurrent(runId)) return

      // 是否为本次会话开启「流式实时显示」：直接读设置（单一真源，避免录音器缓存过期），
      // 结合本次 start 快照兜底，并要求当前供应商在当前配置下真正就绪（qwen 需 WorkspaceId）。
      const settingOn = Boolean(await getSetting('streamingDisplayEnabled', false))
      if (!this.isRunCurrent(runId)) return
      const realtime = (settingOn || Boolean(startOpts.streamingDisplay)) && isStreamingDisplayReady(asrProvider, qwenWorkspaceId)
      addRuntimeEvent('info', 'cloud_api', 'Streaming display decision', { asrProvider, settingOn, startOpt: Boolean(startOpts.streamingDisplay), hasWorkspace: Boolean(qwenWorkspaceId), realtime, runId })
      if (realtime) {
        // 先订阅中间结果，避免建连后、订阅前丢帧
        await this.subscribePartials(runId)
        if (!this.isRunCurrent(runId)) return
      }

      if (asrProvider === 'doubao_v2') {
        // 豆包流式
        this.isDoubaoStream = true
        const asrApiKey = await getSetting('cloudAsr.apiKey', '') as string
        const asrAppId = await getSetting('cloudAsr.appId', '') as string
        if (!this.isRunCurrent(runId)) return

        addRuntimeEvent('info', 'cloud_api', 'Doubao streaming: connecting', { realtime })
        const opened = await this.invokeNativeOpen(runId, 'doubao_stream_open', {
          config: { provider: 'doubao_v2', api_key: asrApiKey, app_id: asrAppId },
          sampleRate: 16000,
          hotwords: startOpts.hotwords ?? [],
          realtime,
        })
        if (!opened || !this.isRunCurrent(runId)) return
        this.doubaoStreamReady = true
        addRuntimeEvent('info', 'cloud_api', 'Doubao streaming: ready')
      } else if (isQwenStreamProvider(asrProvider) && realtime) {
        // 千问流式：只在开启实时显示、且这份配置真的就绪时才走流式 WebSocket。
        //   · qwen_realtime      需要 WorkspaceId（isStreamingDisplayReady 已把关）
        //   · qwen_audio_stream  不需要，填了密钥就能用
        // qwen3-asr-flash（非实时）与未开启实时显示时都不进此分支，走下面的一次性识别。
        this.isQwenStream = true
        this.qwenCommands = QWEN_STREAM_COMMANDS[asrProvider]
        const asrApiKey = await getSetting('cloudAsr.apiKey', '') as string
        if (!this.isRunCurrent(runId)) return

        addRuntimeEvent('info', 'cloud_api', 'Qwen streaming: connecting', { asrProvider, hasWorkspace: Boolean(qwenWorkspaceId) })
        const opened = await this.invokeNativeOpen(runId, this.qwenCommands.open, {
          config: { provider: asrProvider, api_key: asrApiKey, app_id: '' },
          hotwords: startOpts.hotwords ?? [],
          realtime,
          workspaceId: qwenWorkspaceId,
        })
        if (!opened || !this.isRunCurrent(runId)) return
        this.qwenStreamReady = true
        addRuntimeEvent('info', 'cloud_api', 'Qwen streaming: ready', { asrProvider })
      } else {
        // 其他情况（含未配置 WorkspaceId 的千问）走录完再发的一次性识别
        this.teardownPartials()
        return
      }

      // 补发建连期间已缓存的音频
      await this.flushPendingChunks(runId)
      if (!this.isRunCurrent(runId)) return

      // 启动定时器，每 200ms 批量发送一次（收尾阶段不再触发新发送）
      this.flushTimer = setInterval(() => {
        if (!this.isRunCurrent(runId) || this.streamFinishing) return
        const ready = this.doubaoStreamReady || this.qwenStreamReady
        if (ready && this.pendingChunks.length > 0) {
          void this.flushPendingChunks(runId)
        }
      }, 200)
    } catch (err) {
      if (!this.isRunCurrent(runId)) return
      addRuntimeEvent('warn', 'cloud_api', 'Streaming connection failed; falling back to buffered upload', { error: String(err) })
      this.isDoubaoStream = false
      this.isQwenStream = false
      this.doubaoStreamReady = false
      this.qwenStreamReady = false
      this.qwenCommands = null
      this.teardownPartials()
    }
  }

  /** 把一次批量发送排入串行链路，返回可 await 的 Promise。
   *  链式串行保证多次 flush、以及收尾前的最终 flush 都严格按顺序送达 Rust，
   *  绝不会出现音频包穿插到负包之后。 */
  private flushPendingChunks(runId: number): Promise<void> {
    const run = async () => {
      if (!this.isRunCurrent(runId) || this.pendingChunks.length === 0) return
      const chunks = this.pendingChunks
      this.pendingChunks = []

      const totalLen = chunks.reduce((s, c) => s + c.byteLength, 0)
      const merged = new Uint8Array(totalLen)
      let offset = 0
      for (const chunk of chunks) {
        merged.set(new Uint8Array(chunk), offset)
        offset += chunk.byteLength
      }

      const b64 = uint8ArrayToBase64(merged)
      try {
        if (this.isDoubaoStream) {
          await invoke('doubao_stream_send', { pcmB64: b64 })
        } else if (this.isQwenStream && this.qwenCommands) {
          await invoke(this.qwenCommands.send, { pcmB64: b64 })
        }
      } catch (err) {
        addRuntimeEvent('warn', 'cloud_api', 'Streaming send failed', { error: String(err) })
      }
    }
    // 接到发送链尾部，串行执行（无论前一个成功或失败都继续）
    this.sendLock = this.sendLock.then(run, run)
    return this.sendLock
  }

  // ── 处理逻辑 ──

  private async runProcess(
    runId: number,
    startOpts: Readonly<StartOptions>,
  ): Promise<void> {
    const stopTime = performance.now() // stop 时刻，用于计算流式模式的等待时间
    const startTime = this.streamStartTime || stopTime

    try {
      if (!this.isRunCurrent(runId)) return
      const totalBytes = this.pcmBuffers.reduce((sum, buf) => sum + buf.byteLength, 0)
      if (totalBytes === 0) {
        this.teardownPartials()
        this.callbacks.onDone?.()
        return
      }

      const durationSec = (totalBytes / 2) / 16000
      if (durationSec < 0.3) {
        addRuntimeEvent('info', 'cloud_api', 'Audio too short; skipped processing', { durationSec })
        this.teardownPartials()
        this.callbacks.onDone?.()
        return
      }

      // 读取 ASR 配置
      const asrProvider = await getSetting('cloudAsr.provider', 'doubao') as string
      if (!this.isRunCurrent(runId)) return
      const isQwenOmni = isQwenOmniProvider(asrProvider)

      let asrText = ''
      let asrMs = 0

      if ((this.isDoubaoStream && this.doubaoStreamReady) || (this.isQwenStream && this.qwenStreamReady)) {
        // 流式收尾：先置收尾标志（阻止新音频入队/定时器再发），停定时器，
        // flush 剩余数据并等发送链彻底排空，最后再发负包——保证负包是最后一个包。
        this.streamFinishing = true
        if (this.flushTimer) { clearInterval(this.flushTimer); this.flushTimer = null }
        await this.flushPendingChunks(runId)
        await this.sendLock
        if (!this.isRunCurrent(runId)) return

        if (this.isDoubaoStream) {
          addRuntimeEvent('info', 'cloud_api', 'Doubao streaming: sending finish')
          const finishStart = performance.now()
          const text = await this.invokeNativeFinish(runId, 'doubao_stream_finish')
          if (text === undefined || !this.isRunCurrent(runId)) return
          asrText = text
          asrMs = Math.round(performance.now() - finishStart)
          addRuntimeEvent('info', 'cloud_api', 'Doubao streaming: recognition complete', { asrMs, textLen: asrText.length })
        } else {
          // 命令组在 qwenStreamReady 之前就已赋值，这里只是让类型收窄；
          // 真为空就抛出去，而不是悄悄返回空文本（空结果会被显示成「没听到声音」）。
          const commands = this.qwenCommands
          if (!commands) throw new Error('Qwen streaming session has no native command set')
          addRuntimeEvent('info', 'cloud_api', 'Qwen streaming: sending finish', { command: commands.finish })
          const finishStart = performance.now()
          const text = await this.invokeNativeFinish(runId, commands.finish)
          if (text === undefined || !this.isRunCurrent(runId)) return
          asrText = text
          asrMs = Math.round(performance.now() - finishStart)
          addRuntimeEvent('info', 'cloud_api', 'Qwen streaming: recognition complete', { asrMs, textLen: asrText.length })
        }
      } else {
        // 非豆包 / 豆包建连失败：录完再发
        const merged = new Uint8Array(totalBytes)
        let offset = 0
        for (const buf of this.pcmBuffers) {
          merged.set(new Uint8Array(buf), offset)
          offset += buf.byteLength
        }
        const audioB64 = uint8ArrayToBase64(merged)

        const asrApiKey = await getSetting('cloudAsr.apiKey', '') as string
        const asrAppId = await getSetting('cloudAsr.appId', '') as string
        if (!this.isRunCurrent(runId)) return
        const qwenOmniModel = resolveQwenOmniModel(asrProvider)

        let omniInstructions: string | undefined
        if (isQwenOmni) {
          const savedPrompt = await getSetting('cloudAsr.omniSystemPrompt', '') as string
          if (!this.isRunCurrent(runId)) return
          omniInstructions = savedPrompt || undefined
        }

        const asrConfig: AsrProviderConfig = {
          provider: isQwenOmni ? 'qwen_omni' : asrProvider,
          api_key: asrApiKey,
          app_id: asrAppId,
          ...(isQwenOmni && {
            extra: { model: qwenOmniModel, instructions: omniInstructions },
          }),
        }

        if (asrProvider === 'openai_compat') {
          asrConfig.extra = {
            api_url: await getSetting('cloudAsr.apiUrl', ''),
            model: await getSetting('cloudAsr.model', ''),
            language: startOpts.language,
          }
          if (!this.isRunCurrent(runId)) return
        }
        addRuntimeEvent('info', 'cloud_api', 'ASR started', { provider: asrProvider, durationSec })
        const asrResult = await invoke<AsrResult>('cloud_transcribe', {
          request: {
            audio_b64: audioB64,
            sample_rate: 16000,
            asr_config: asrConfig,
            hotwords: startOpts.hotwords ?? [],
          },
        })
        if (!this.isRunCurrent(runId)) return
        asrText = asrResult.text
        asrMs = asrResult.elapsed_ms
      }

      this.pcmBuffers = []
      // ASR 已拿到最终文本，后续不再有中间结果，撤下监听
      this.teardownPartials()

      // 还原被 ASR 拆开加空格的无空格热词（如豆包把 "SayIt" 识别成 "Say It"）
      asrText = restoreHotwordSpacing(asrText, startOpts.hotwords ?? [])

      // 发送 ASR 中间结果
      this.callbacks.onASR?.({ text: asrText, asrMs, durationSec })

      if (!asrText.trim()) {
        this.callbacks.onFinal?.({ asrText: '', llmText: '', asrMs, llmMs: 0, durationSec })
        this.callbacks.onDone?.()
        return
      }

      // AI 校对（Qwen Omni 已内置 AI，跳过）
      const polish = isQwenOmni
        ? {
          llmText: startOpts.textContext?.selectedText || asrText,
          llmMs: 0,
          contextApplied: startOpts.textContext ? false : undefined,
          aiSource: 'none' as const,
          aiStatus: 'skipped' as const,
        }
        : await polishWithClientAi({
          asrText,
          durationSec,
          startOptions: startOpts,
          logSource: 'cloud_api',
          isCurrent: () => this.isRunCurrent(runId),
        })
      if (!polish || !this.isRunCurrent(runId)) return

      const totalMs = Math.round(performance.now() - startTime)
      addRuntimeEvent('info', 'cloud_api', 'Processing complete', {
        durationSec,
        asrMs,
        llmMs: polish.llmMs,
        totalMs,
        runId,
      })

      const omniModel = isQwenOmni ? resolveQwenOmniModel(asrProvider) : undefined

      this.callbacks.onFinal?.({
        asrText,
        asrMs,
        durationSec,
        ...polish,
        ...(isQwenOmni && { asrEngine: 'qwen_omni', asrModel: omniModel }),
      })
      this.callbacks.onDone?.()
    } catch (err) {
      if (!this.isRunCurrent(runId)) return
      addRuntimeEvent('error', 'cloud_api', 'Processing failed', { error: String(err) })
      this.teardownPartials()
      this.callbacks.onError?.(String(err))
      this.callbacks.onDone?.()
    } finally {
      this.completeRun(runId)
    }
  }
}
