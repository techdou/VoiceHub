import * as bridge from '../bridge'
import { REMOTE_MIC_ID } from '../remoteCapture'
import { RemoteTransport } from './RemoteTransport'
import { startCapture, stopCapture } from '../audio'
import { getProvider, type TranscriptionProvider, type TranscriptionCallbacks, type FinalResult } from '../transcription'
import { isStreamingDisplayReady } from '@/lib/asrModels'
import {
  addHistory,
  deleteHistory,
  getActivePresetId,
  getPromptPresets,
  getSetting,
  setActivePresetId,
  type HistoryFailReasonCode,
  type PromptPreset,
} from '../store'
import { setActivePresetKnown } from '../../stores/activePreset'
import { addRuntimeEvent } from '../debugLog'
import { saveRecordingAudio } from '../audioFileService'
import {
  BUILTIN_SET_ACTIVE_KEY,
  BUILTIN_SET_WORDS_KEY,
  CUSTOM_THEME_ACTIVE_KEY,
  CUSTOM_THEMES_KEY,
  composeHotwords,
  normalizeBuiltinSetActive,
  normalizeBuiltinSetWords,
  normalizeCustomThemeActive,
  normalizeCustomThemes,
} from '../hotwords/model'
import { invoke } from '@tauri-apps/api/core'
import { createWaveformBarState, computeBarsFromPCM, resetWaveformBarState } from '../waveform'
import { elapsedSecFromPerf } from '../timeModel'
import {
  captureActiveInsertionTarget,
  clearCapturedInsertionTarget,
  startInsertionTargetTracking,
  stopInsertionTargetTracking,
} from '../textInsertion'
import { applyTextTransforms } from '../textPostProcess'
import type { ActiveAppContext } from '../../types/appContext'
import type { ClientRuntimeInfo } from '../../types/appApi'
import { OverlayService } from './OverlayService'
import { PasteService, type ProbeResult } from './PasteService'
import { createDefaultUserStats } from '../personalization/defaults'
import { resolvePromptRouting } from '../personalization/promptRouter'
import {
  getAppPromptRules,
  getUserStats,
  recordSessionStats,
} from '../personalization/store'
import type { AppPromptRule, PromptResolution, UserStats } from '../personalization/types'
import type { PTTEventPayload, RecorderState } from './types'
import { MAX_RECORDING_SEC, RECORDING_COUNTDOWN_SEC } from './types'
import {
  summarizeAppContext as _summarizeAppContext,
  buildStatsAppId as _buildStatsAppId,
  isModifierPTTSetting as _isModifierPTTSetting,
  computeProcessingTimeoutMs as _computeProcessingTimeoutMs,
  classifyMicLevel,
  judgeOsMicMute,
  type MicLevel,
} from './helpers'
import { t } from '@/i18n'
import { describeProviderError } from '@/lib/errorMessages'
import { describeMicSource, micSourceChanged } from './micSourceReminder'
import {
  CONTEXT_SELECTION_EDIT_PROMPT,
  CONTEXT_SELECTION_EDIT_PROMPT_SETTING_KEY,
  normalizeContextSelectionEditPrompt,
  resolveContextAwareOutput,
  usableTextContext,
  withContextAwareInstructions,
} from '../contextAware'

type StateTransition =
  | ['idle', 'recording']
  | ['recording', 'processing']
  | ['recording', 'idle']
  | ['processing', 'idle']

const VALID_TRANSITIONS: StateTransition[] = [
  ['idle', 'recording'],
  ['recording', 'processing'],
  ['recording', 'idle'],
  ['processing', 'idle'],
]

const LATE_FINAL_GRACE_MS = 15000
const MODIFIER_PTT_RELEASE_GUARD_MS = 200
const MIC_MUTED_AUTO_CANCEL_MS = 3000

function classifyHistoryProviderFailure(message: string): HistoryFailReasonCode {
  const code = describeProviderError(message).code
  switch (code) {
    case 'provider_timeout':
    case 'provider_unreachable':
    case 'provider_bad_key':
    case 'provider_forbidden':
    case 'provider_rate_limit':
    case 'provider_no_model':
      return code
    default:
      return 'provider_failed'
  }
}

interface TimedOutProcessingContext {
  runId: number
  timedOutAt: number
  audioDurationSec: number
  wallTimeSec: number
  promptResolution: PromptResolution | null
  appContext: ActiveAppContext | null
  audioChunks: ArrayBuffer[]
  probeResult: ProbeResult | null
}

interface ResetToIdleOptions {
  keepOverlay?: boolean
  preserveLateFinalContext?: boolean
}

export class RecorderOrchestrator {
  private remoteSession = false
  private readonly remoteTransport = new RemoteTransport(this)

  async startRemoteRecording(): Promise<boolean> {
    if (this.remoteSession || this.state !== 'idle' || this.startRecordingLock || this.textInsertionInFlight || this.finalizingLateRunId) {
      this.overlayService.showError('VoiceHub: recorder is busy')
      return false
    }
    this.remoteSession = true
    this.handsFreeMode = false
    try {
      await this.startRecording()
      if (this.getState() === 'recording') return true
      this.remoteSession = false
      return false
    } catch (error) {
      this.remoteSession = false
      throw error
    }
  }

  async stopRemoteRecording(error?: string | null) {
    if (!this.remoteSession) return
    try {
      if (error) {
        await this.cancelRecording(this.activeRunId)
        this.overlayService.showError(error)
      } else { await this.stopRecording() }
    } finally { this.remoteSession = false }
  }
  private state: RecorderState = 'idle'
  private onStateChange: ((s: RecorderState) => void) | null = null
  private initialized = false
  private runSequence = 0
  private activeRunId = 0

  private recordStartPerf = 0
  private audioSentSamples = 0
  /** Wall time captured at stopRecording — used for history durationSec so it
   *  matches what the user saw on the overlay (not inflated by backend latency). */
  private wallTimeAtStopSec = 0
  private finalHandledInCurrentRun = false
  private textInsertionInFlight = false
  /** 迟到 final 正在完成历史/插入；此期间禁止新录音复用 Provider。 */
  private finalizingLateRunId = 0
  /** Esc 仅在 processing 的可逆阶段允许取消；进入系统粘贴前永久关闭。 */
  private processingCancelable = false
  /** 当前 fallback 卡片对应的 run token；用于拒绝队列中迟到的旧 dismiss。 */
  private activeFallbackToken = 0
  /** Guard against re-entrant startRecording calls during async setup */
  private startRecordingLock = false
  /** PTT up arrived while startRecording was still initializing — stop immediately after setup */
  private pendingStopWhileStarting = false
  private processingTimeoutId: ReturnType<typeof setTimeout> | null = null
  private finalReceivedAt = 0
  private timedOutProcessingContext: TimedOutProcessingContext | null = null
  private pendingHistoryArtifact: { runId: number; recordId: string; audioFilePath?: string } | null = null

  private handsFreeMode = false
  private pttSuppressed = false
  private lastToggleTime = 0
  private lastPTTUpAt = 0
  private lastPTTUpUsedModifier = false

  private cachedMicId = ''
  /** Last successfully opened input route in this app process. Kept in memory on purpose. */
  private lastMicSourceIdentity: string | null = null
  /**
   * 是否让浏览器对麦克风做降噪。默认开（底噪大的机器需要），
   * 但降噪是按"人听得舒服"优化的，可能削掉 ASR 要的细节，所以可关。
   */
  private noiseSuppression = true
  /** 是否在录音期间静音系统输出（防外放被麦克风回采）。默认关闭。 */
  private cachedMuteSystemAudio = false
  /** 插入文本后是否自动还原剪贴板为插入前内容。默认开启。 */
  private cachedProtectClipboard = true
  /** 标记本次录音是否已施加系统静音，用于配对恢复。 */
  private systemMuteApplied = false
  /** 延迟静音定时器（等提示音播完再静音）。 */
  private systemMuteTimerId: ReturnType<typeof setTimeout> | null = null
  private cachedPresets: PromptPreset[] = []
  private cachedActivePresetId = 'intent'
  private cachedAiEnabled = true
  /** 0 = 所有语音都用 AI；正数 = 达到门槛才调用 AI。 */
  private cachedAiMinDurationSec = 0
  /** Reads bounded editor text only when explicitly enabled; default is false. */
  private cachedContextAwareWriting = false
  /** User-visible selection-edit prompt. Cached with other recording settings. */
  private cachedContextSelectionEditPrompt = CONTEXT_SELECTION_EDIT_PROMPT
  private cachedClientRuntimeInfo: ClientRuntimeInfo | null = null
  /** 客户端运行时信息是否已成功采集一次。它是进程内静态元数据（区域/内存/主机名等），
   *  只需启动时取一次，不该挂在每次改设置都触发的刷新热路径上。失败时保持 false 以便重试。 */
  private clientRuntimeInfoLoaded = false
  private cachedAppPromptRules: AppPromptRule[] = []
  private cachedUserStats: UserStats = createDefaultUserStats()
  private cachedHotwords: string[] = []
  /** 是否把热词注入 AI 提示词（默认关，用户可在热词页开启）。 */
  private cachedInjectHotwords = false
  private cachedLanguage: string = ''
  /** 是否开启流式实时显示（识别过程中把中间结果实时显示在悬浮窗）。默认关闭。 */
  private cachedStreamingDisplay = false
  private currentActiveAppContext: ActiveAppContext | null = null
  private currentPromptResolution: PromptResolution | null = null
  /** Probe result captured at startRecording time (before audio capture begins).
   *  Used by handleTextInsertion so we inject into the window that was focused
   *  when the user started speaking, not whatever happens to be focused later. */
  private cachedProbeResult: import('./PasteService').ProbeResult | null = null

  private readonly overlayWaveState = createWaveformBarState()
  private readonly overlayService = new OverlayService(() => this.getLiveElapsedSec())
  private readonly pasteService = new PasteService()
  private get provider(): TranscriptionProvider { return getProvider() }
  private recordedChunks: ArrayBuffer[] = []
  private captureReadyPromise: Promise<void> | null = null
  /** 5-minute auto-stop timer for hands-free mode */
  private handsFreeAutoStopId: ReturnType<typeof setTimeout> | null = null
  /** 系统确认麦克风静音后的自动取消计时器，避免悬浮窗无限停留。 */
  private micMutedAutoCancelId: ReturnType<typeof setTimeout> | null = null
  /** 静音探测代次：实际打开设备的复核必须覆盖较早发起的预检查结果。 */
  private micMuteProbeSequence = 0
  /** 连续「非正常音量」（静音或偏低）的采样数 */
  private consecutiveSilentSamples = 0
  /** 当前连续非正常音量的采样数，仅用于判断正常说话候选之间的间隔。 */
  private consecutiveNonVoicedSamples = 0
  /** 当前这段安静里是否出现过任何非零采样。
   * 只要为真，就只能提示「请靠近麦克风」，不能提示「未检测到声音」。 */
  private quietRunSawSignal = false
  /** 连续正常音量的采样数——用于「持续说话一段时间才清除警告」的迟滞判断，避免瞬时小声音把警告瞬间清掉造成闪烁 */
  private consecutiveVoicedSamples = 0
  /** 悬浮窗当前正在显示的音量类警告 */
  private currentVolumeWarning: 'none' | 'muted' | 'low' = 'none'
  /** 本次录音是否已经听到过一次正常音量的说话。一旦听到过，说明麦克风正常，
   *  后续安静只是正常停顿思考——只给温和「请靠近麦克风」，绝不再报「未检测到声音」造成惊吓。 */
  private hasDetectedVoiceThisSession = false
  private lastLowVolumeWarnAt = 0
  /** 麦克风被静音已被「系统标志 + 音频信号」双重证实。为真时悬浮窗显示红色高警，
   *  并抑制基于信号的琥珀提醒，直到检测到真实说话（用户中途取消了静音）。 */
  private osMicMuted = false
  /** 系统报了「端点被静音」，但还没被音频信号证实。
   *
   *  为什么不能直接相信系统：IAudioEndpointVolume::GetMute 反映的是端点上的静音**设置**，
   *  不等于采集流真被切断。实测 Plantronics Blackwire 5220 USB 耳麦（软/硬静音会与 Windows
   *  端点标志同步）会停在 GetMute=true，而 getUserMedia 照常收到正常音量的音频 —— 于是每次
   *  按热键都先弹一次红色高警「麦克风已被静音」，说话 0.5s 后又自己消失。
   *  所以这个标志只当线索：必须等 PCM 证实真的全 0 才升级成 osMicMuted。 */
  private pendingOsMicMuted = false
  /** pending 期间累计的全 0 采样数，达到 OS_MIC_MUTE_CONFIRM_SAMPLES 才确认。 */
  private pendingOsMicMutedSamples = 0

  /** Audio stats tracking */
  private audioStatsRmsSum = 0
  private audioStatsPeakRms = 0
  private audioStatsPeakAmplitude = 0
  private audioStatsSilentFrames = 0
  private audioStatsTotalFrames = 0
  private static readonly SILENCE_RMS_THRESHOLD = 0.01

  setStateListener(cb: (s: RecorderState) => void) {
    this.onStateChange = cb
  }

  getState() {
    return this.state
  }

  private isRunCurrent(runId: number) {
    return runId !== 0 && this.activeRunId === runId
  }

  /** 幂等结束指定代次；所有清理都带 runId 条件，绝不清除随后创建的新代次。 */
  private finishRun(runId: number) {
    if (runId === 0) return
    if (this.pendingHistoryArtifact?.runId === runId) {
      this.pendingHistoryArtifact = null
    }
    if (this.timedOutProcessingContext?.runId === runId) {
      this.timedOutProcessingContext = null
    }
    if (this.activeRunId === runId) {
      this.activeRunId = 0
      this.processingCancelable = false
    }
  }

  /** 若已开启「录音时静音系统声音」，在就绪提示音播放之后再静音系统输出。
   *  延迟略长于提示音时长（~150ms），避免把提示音也一起静掉。 */
  private scheduleSystemMuteIfEnabled() {
    if (!this.cachedMuteSystemAudio) return
    this.clearSystemMuteTimer()
    this.systemMuteTimerId = setTimeout(() => {
      this.systemMuteTimerId = null
      // 若此时已不在录音（短按等），不再静音，避免残留
      if (this.state !== 'recording') return
      this.applySystemMute()
    }, 250)
  }

  private applySystemMute() {
    if (this.systemMuteApplied) return
    this.systemMuteApplied = true
    void bridge.muteSystemOutput().catch((e) => {
      this.systemMuteApplied = false
      addRuntimeEvent('warn', 'recorder', 'Failed to mute system audio', { error: String(e) })
    })
  }

  private clearSystemMuteTimer() {
    if (this.systemMuteTimerId) {
      clearTimeout(this.systemMuteTimerId)
      this.systemMuteTimerId = null
    }
  }

  /** 恢复系统输出到静音前的状态，并清除挂起的静音定时器。 */
  private restoreSystemMuteIfNeeded() {
    this.clearSystemMuteTimer()
    if (!this.systemMuteApplied) return
    this.systemMuteApplied = false
    void bridge.restoreSystemOutput().catch((e) => {
      addRuntimeEvent('warn', 'recorder', 'Failed to restore system audio', { error: String(e) })
    })
  }

  /** 未就绪时的简短提示文案（按当前工作模式区分）。 */
  private notReadyMessage(): string {
    switch (this.provider.mode) {
      case 'server':
        return t('recorder.serverDisconnected')
      case 'local':
        return t('recorder.modelNotDownloaded')
      default:
        return t('recorder.serviceNotReady')
    }
  }

  /** 通过快捷键切换当前润色模式，并在空闲时用悬浮窗提示。 */
  private async handlePresetSwitch(presetId: string) {
    try {
      // 优先用已缓存的预设列表，避免 IPC 等待导致的延时
      let target = this.cachedPresets.find((p) => p.id === presetId)
      if (!target) {
        const presets = await getPromptPresets()
        target = presets.find((p) => p.id === presetId)
        this.cachedPresets = presets
      }
      if (!target) {
        addRuntimeEvent('warn', 'recorder', 'Failed to switch cleanup preset: preset not found', { presetId })
        return
      }
      // 立即生效：更新录音器缓存 + 通知 UI + 悬浮窗提示（均无需等待 IPC）
      this.cachedActivePresetId = presetId
      setActivePresetKnown(presetId, target.name)
      if (this.state === 'idle') {
        this.overlayService.showPresetSwitched(target.name)
      }
      addRuntimeEvent('info', 'recorder', 'Cleanup preset switched by shortcut', { presetId, name: target.name })
      // 持久化放到后台，不阻塞 UI 反馈
      void setActivePresetId(presetId)
    } catch (error) {
      addRuntimeEvent('error', 'recorder', 'Cleanup preset switch failed', { error: String(error) })
    }
  }

  /** 临时禁用/启用 PTT（用于欢迎向导热键确认步骤） */
  setPttSuppressed(suppressed: boolean) {
    this.pttSuppressed = suppressed
  }

  async init() {
    if (this.initialized) return
    this.initialized = true

    startInsertionTargetTracking()
    await this.refreshRuntimeSettings()
    // 静态机器信息只在启动采集一次；不 await，避免拖慢建连（clientMeta 可空，采集期间为 null 无碍）。
    void this.ensureClientRuntimeInfo()
    this.ensureConnection()

    this.remoteTransport.start()
    void bridge.listen<string>('voicehub-voice-error', event => this.overlayService.showError(event.payload))

    // 快捷键切换润色模式（由 Rust global_shortcut 触发）
    void bridge.listen('switch-preset', (event: unknown) => {
      const payload = (event as { payload?: { presetId?: string } })?.payload
      const presetId = payload?.presetId
      if (presetId) void this.handlePresetSwitch(presetId)
    })

    bridge.onEscapeAction(({ mode, token }) => {
      if (mode === 'cancel_recording') {
        // Esc 可能在录音 stop 已发起、事件尚在 dispatcher 队列时到达；若此时已进入
        // 可逆 processing，沿用同一 run token 取消，避免用户明明按了却被竞态吞掉。
        if (this.state === 'processing' && this.processingCancelable) {
          this.cancelProcessing(token)
        } else {
          void this.cancelRecording(token)
        }
        return
      }
      if (mode === 'cancel_processing') {
        this.cancelProcessing(token)
        return
      }
      if (mode === 'dismiss_fallback') {
        if (token === 0 || token !== this.activeFallbackToken) {
          addRuntimeEvent('info', 'recorder', 'Ignored stale Esc fallback dismissal', {
            token,
            activeFallbackToken: this.activeFallbackToken,
          })
          return
        }
        addRuntimeEvent('info', 'recorder', 'Fallback card dismissed with Esc', { token })
        this.activeFallbackToken = 0
        this.overlayService.hide()
      }
    })

    bridge.onPTTDown((payload) => {
      if (this.remoteSession) return
      this.notePTTDown(payload)
      this.logPTTEvent('down', payload)
      if (this.pttSuppressed || this.handsFreeMode) {
        addRuntimeEvent('info', 'ptt', 'event:down ignored', {
          ...this.getPTTEventContext(payload),
          ignoreReason: this.pttSuppressed ? 'ptt_suppressed' : 'hands_free_mode',
        })
        return
      }
      if (this.state === 'idle') {
        addRuntimeEvent('info', 'ptt', 'event:down accepted -> startRecording', this.getPTTEventContext(payload))
        void this.startRecording()
        return
      }
      addRuntimeEvent('info', 'ptt', 'event:down ignored', {
        ...this.getPTTEventContext(payload),
        ignoreReason: 'state_not_idle',
      })
    })

    bridge.onPTTUp((payload) => {
      if (this.remoteSession) return
      this.notePTTUp(payload)
      this.logPTTEvent('up', payload)
      if (this.pttSuppressed || this.handsFreeMode) {
        addRuntimeEvent('info', 'ptt', 'event:up ignored', {
          ...this.getPTTEventContext(payload),
          ignoreReason: this.pttSuppressed ? 'ptt_suppressed' : 'hands_free_mode',
        })
        return
      }
      if (this.state === 'recording') {
        addRuntimeEvent('info', 'ptt', 'event:up accepted -> stopRecording', this.getPTTEventContext(payload))
        void this.stopRecording()
        return
      }
      // PTT up arrived while startRecording is still initializing (state is still 'idle')
      if (this.startRecordingLock) {
        addRuntimeEvent('info', 'ptt', 'event:up deferred — startRecording in progress', this.getPTTEventContext(payload))
        this.pendingStopWhileStarting = true
        return
      }
      addRuntimeEvent('info', 'ptt', 'event:up ignored', {
        ...this.getPTTEventContext(payload),
        ignoreReason: 'state_not_recording',
      })
    })

    // ptt-toggle 监听已移除：native 从不发送该事件（上游遗留死监听），
    // 免提切换走 toggle-hands-free（下方）。

    bridge.onToggleHandsFree((payload) => {
      if (this.remoteSession) return
      this.logPTTEvent('hands_free', payload)
      if (this.pttSuppressed) {
        addRuntimeEvent('info', 'ptt', 'event:hands_free ignored', {
          ...this.getPTTEventContext(payload),
          ignoreReason: 'ptt_suppressed',
        })
        return
      }
      addRuntimeEvent('info', 'ptt', 'event:hands_free accepted', this.getPTTEventContext(payload))
      this.pttToggle(true)
    })

    // 9-minute warning from Rust keyboard hook (PTT hold mode)
    bridge.onPTTTimeoutWarning(() => {
      if (this.state === 'recording') {
        addRuntimeEvent('warn', 'recorder', 'Recording is approaching the five-minute limit')
        this.overlayService.showTimeoutWarning()
      }
    })
  }

  cleanup() {
    this.remoteTransport.stop()
    this.clearProcessingTimeout()
    this.clearMicMutedAutoCancelTimer()
    this.micMuteProbeSequence++
    this.overlayService.dispose()
    stopInsertionTargetTracking()
    void stopCapture().catch(() => { })
    this.provider.disconnect()
  }

  /** 仅更新 AI 整理开关的缓存值，无 IPC/热词/语言等全量刷新。
   *  供标题栏/设置页的开关按钮使用，避免快速切换时卡顿。 */
  setAiEnabledCache(next: boolean) {
    this.cachedAiEnabled = next
  }

  showAiEnabledToast(enabled: boolean) {
    if (this.state === 'idle') this.overlayService.showAiCleanupToggled(enabled)
  }

  /** 仅更新当前润色模式（preset）的缓存值，无 IPC/热词/语言等全量刷新。
   *  供设置页点击切换润色模式使用，避免快速切换时卡顿。 */
  setActivePresetCache(id: string) {
    this.cachedActivePresetId = id
  }

  /** 新建、编辑或删除预设后同步最新列表，避免 active id 指向尚未进入运行时缓存的新预设。 */
  setPromptPresetsCache(presets: PromptPreset[]) {
    this.cachedPresets = presets.map((preset) => ({ ...preset }))
  }

  /** 热词变更后同步下一次录音使用的快照；当前录音仍使用开始时已捕获的 StartOptions。 */
  setHotwordsCache(words: string[]) {
    this.cachedHotwords = Array.from(new Set(words.map((word) => word.trim()).filter(Boolean)))
  }

  /** 仅更新「流式实时显示」开关缓存，供外观设置切换后立即生效。 */
  setStreamingDisplayCache(next: boolean) {
    this.cachedStreamingDisplay = next
  }

  /** 录音开始时实时判定是否激活流式字幕气泡（读最新的开关/供应商/WorkspaceId，避免缓存过期）。
   *  判据统一交给 isStreamingDisplayReady：doubao_v2 与 qwen_audio_stream 直接可用，
   *  qwen_realtime 还要配 WorkspaceId；qwen3-asr-flash 等非流式模型不激活。 */
  private async applyStreamingActive(): Promise<void> {
    try {
      const [streamOn, provider, workspaceId] = await Promise.all([
        getSetting('streamingDisplayEnabled', false),
        getSetting('cloudAsr.provider', 'doubao_v2'),
        getSetting('cloudAsr.qwen.workspaceId', ''),
      ])
      if (this.state !== 'recording') return
      const active = Boolean(streamOn)
        && this.provider.mode === 'cloud_api'
        && isStreamingDisplayReady(String(provider || ''), String(workspaceId || ''))
      this.overlayService.setStreamingActive(active)
    } catch {
      /* ignore — 气泡占位是锦上添花，读取失败不影响录音 */
    }
  }

  async refreshRuntimeSettings() {
    const [
      micId,
      muteSystemAudio,
      protectClipboard,
      presets,
      activePresetId,
      aiEnabled,
      aiMinDurationSec,
      contextAwareWriting,
      contextSelectionEditPrompt,
      appPromptRules,
      userStats,
      streamingDisplay,
      injectHotwords,
      noiseSuppression,
    ] = await Promise.all([
      getSetting('selectedMic', ''),
      getSetting('muteSystemAudioWhileRecording', false),
      getSetting('protectClipboard', true),
      getPromptPresets(),
      getActivePresetId(),
      getSetting('aiEnabled', false),
      getSetting('aiMinDurationSec', 0),
      getSetting('contextAwareWritingEnabled', false),
      getSetting(CONTEXT_SELECTION_EDIT_PROMPT_SETTING_KEY, CONTEXT_SELECTION_EDIT_PROMPT),
      getAppPromptRules(),
      getUserStats(),
      getSetting('streamingDisplayEnabled', false),
      getSetting('injectHotwordsToPrompt', false),
      getSetting('micNoiseSuppression', true),
    ])

    this.noiseSuppression = Boolean(noiseSuppression)
    this.cachedMicId = String(micId || '')
    this.cachedMuteSystemAudio = Boolean(muteSystemAudio)
    this.cachedProtectClipboard = Boolean(protectClipboard)
    this.cachedInjectHotwords = Boolean(injectHotwords)
    this.cachedPresets = presets
    this.cachedActivePresetId = activePresetId
    this.cachedAiEnabled = Boolean(aiEnabled)
    this.cachedAiMinDurationSec = Math.max(0, Math.min(MAX_RECORDING_SEC, Number(aiMinDurationSec) || 0))
    this.cachedContextAwareWriting = Boolean(contextAwareWriting)
    this.cachedContextSelectionEditPrompt = normalizeContextSelectionEditPrompt(contextSelectionEditPrompt)
    this.cachedAppPromptRules = appPromptRules
    this.cachedUserStats = userStats
    this.cachedStreamingDisplay = Boolean(streamingDisplay)
    await this.overlayService.refreshSettings()

    // 热词 / 服务器语言彼此独立，并行加载而非依次 await，减少本函数的总耗时
    // （每次切换 AI 整理开关等都会触发这里）。
    // 客户端运行时信息（区域/内存/主机名等）是进程内静态元数据，已移出此热路径，
    // 仅在启动时经 ensureClientRuntimeInfo() 采集一次，避免改任意设置都重新采集。
    const [hotwordsResult, languageResult] = await Promise.allSettled([
      Promise.all([
        getSetting(BUILTIN_SET_WORDS_KEY, {}),
        getSetting(BUILTIN_SET_ACTIVE_KEY, {}),
        getSetting(CUSTOM_THEMES_KEY, []),
        getSetting(CUSTOM_THEME_ACTIVE_KEY, {}),
      ]).then(([rawSetWords, rawSetActive, rawCustomThemes, rawCustomThemeActive]) => {
        const setWords = normalizeBuiltinSetWords(rawSetWords as Record<string, unknown>)
        const setActive = normalizeBuiltinSetActive(rawSetActive as Record<string, unknown>)
        const themes = normalizeCustomThemes(rawCustomThemes)
        const themeActive = normalizeCustomThemeActive(rawCustomThemeActive as Record<string, unknown>, themes)
        return composeHotwords([], setWords, setActive, themes, themeActive)
      }),
      getSetting('server.language', 'auto').then((lang) => {
        const l = lang as string
        return l && l !== 'auto' ? l : ''
      }),
    ])

    this.cachedHotwords = hotwordsResult.status === 'fulfilled' ? hotwordsResult.value : []
    this.cachedLanguage = languageResult.status === 'fulfilled' ? languageResult.value : ''
  }

  /** 采集一次客户端运行时信息并缓存。静态元数据，进程内只成功采集一次；
   *  失败不置位 loaded，留待下次重试（避免把 unknown/0 永久缓存）。
   *  clientMeta 本就可空，采集期间为 null 不影响业务。 */
  async ensureClientRuntimeInfo() {
    if (this.clientRuntimeInfoLoaded) return
    try {
      this.cachedClientRuntimeInfo = await bridge.getClientRuntimeInfo()
      this.clientRuntimeInfoLoaded = true
    } catch {
      this.cachedClientRuntimeInfo = null
    }
  }

  /** 工作模式或服务地址变更后强制重连 provider。
   *  先断开旧连接再按新配置连接，确保连接状态与新地址一致（否则改错地址后仍显示"已连接"）。
   *  录音进行中不强制断开，避免打断当前会话。 */
  reconnectProvider() {
    if (this.state === 'idle') {
      this.provider.disconnect()
    }
    this.ensureConnection()
  }

  /** 仅刷新 overlay 显示设置（主题/长度/时长），不触碰录音相关缓存。
   *  用于外观设置页面，避免改个颜色就触发十几个 IPC 调用。 */
  async refreshOverlaySettings() {
    await this.overlayService.refreshSettings()
  }

  // ── State machine ──

  private transition(to: RecorderState): boolean {
    const pair = [this.state, to] as [RecorderState, RecorderState]
    const valid = VALID_TRANSITIONS.some(([f, t]) => f === pair[0] && t === pair[1])
    if (!valid) {
      addRuntimeEvent('warn', 'recorder', `Ignored invalid state transition ${this.state} → ${to}`)
      return false
    }
    addRuntimeEvent('info', 'recorder', 'State changed', { from: this.state, to })
    this.state = to
    this.onStateChange?.(to)
    return true
  }

  private clearProcessingTimeout() {
    if (this.processingTimeoutId) {
      clearTimeout(this.processingTimeoutId)
      this.processingTimeoutId = null
    }
  }

  private clearMicMutedAutoCancelTimer() {
    if (this.micMutedAutoCancelId) {
      clearTimeout(this.micMutedAutoCancelId)
      this.micMutedAutoCancelId = null
    }
  }

  private scheduleMicMutedAutoCancel(runId: number) {
    this.clearMicMutedAutoCancelTimer()
    const closeWhenReady = () => {
      this.micMutedAutoCancelId = null
      if (!this.isRunCurrent(runId) || !this.osMicMuted) return
      if (this.state === 'idle' && this.startRecordingLock) {
        // 防御：这个计时器现在只在收到音频帧后才排，理论上必定已在 recording。
        // 万一将来又从「采集就绪前」的路径调用，也不要在这里直接放弃。
        this.micMutedAutoCancelId = setTimeout(closeWhenReady, 100)
        return
      }
      if (this.state !== 'recording') return
      addRuntimeEvent('info', 'recorder', 'Muted microphone warning timed out; recording will close automatically', {
        runId,
        timeoutMs: MIC_MUTED_AUTO_CANCEL_MS,
      })
      void this.cancelRecording(runId, { showCanceled: false, reason: 'mic_muted_timeout' })
    }
    this.micMutedAutoCancelId = setTimeout(closeWhenReady, MIC_MUTED_AUTO_CANCEL_MS)
  }

  /** 在采集链路初始化前查询系统静音状态；默认设备不经过可能很慢的设备枚举。 */
  private async checkConfiguredMicMuted(runId: number) {
    // 在任何异步操作之前占用代次；之后发起的实际设备复核可以可靠淘汰本次预检查。
    const probeSequence = ++this.micMuteProbeSequence
    // 空值与 Web Media 的 "default" 都表示跟随 Windows 当前默认录音端点。
    // 直接把 null 交给原生层查询该端点的 GetMute；deviceId 只用于定位，不参与静音判定。
    if (!this.cachedMicId || this.cachedMicId === 'default') {
      await this.checkMicMuted(null, runId, probeSequence)
      return
    }

    try {
      const devices = await navigator.mediaDevices.enumerateDevices()
      if (probeSequence !== this.micMuteProbeSequence || !this.isRunCurrent(runId)) return
      const inputs = devices.filter((device) => device.kind === 'audioinput')
      const selected = inputs.find((device) => device.deviceId === this.cachedMicId)

      const label = selected?.label?.trim() || ''
      // 指定设备却无法取得名称时不能回退查系统默认端点，否则可能把另一支麦克风的状态报过来。
      if (!label) return
      await this.checkMicMuted(label, runId, probeSequence)
    } catch {
      // 预检查失败不影响录音；采集真正打开后还会用 MediaStreamTrack.label 再复核一次。
    }
  }

  private getLiveElapsedSec() {
    if (this.state === 'recording' && this.recordStartPerf > 0) {
      return elapsedSecFromPerf(this.recordStartPerf)
    }
    return this.getAudioDurationSec()
  }

  private getAudioDurationSec() {
    return this.audioSentSamples > 0 ? this.audioSentSamples / 16000 : 0
  }

  private resetToIdle(options?: ResetToIdleOptions) {
    addRuntimeEvent('info', 'recorder', 'Reset to idle', {
      fromState: this.state,
      keepOverlay: Boolean(options?.keepOverlay),
      handsFreeMode: this.handsFreeMode,
      textInsertionInFlight: this.textInsertionInFlight,
    })
    this.clearProcessingTimeout()
    this.clearMicMutedAutoCancelTimer()
    this.micMuteProbeSequence++
    if (this.handsFreeAutoStopId) {
      clearTimeout(this.handsFreeAutoStopId)
      this.handsFreeAutoStopId = null
    }
    this.overlayService.stopListeningTicker()
    this.overlayService.resetWarnings()
    this.overlayService.resetStreamingText()
    // 安全网：若仍处于我们施加的系统静音中，确保恢复
    this.restoreSystemMuteIfNeeded()
    this.startRecordingLock = false
    this.pendingStopWhileStarting = false
    this.handsFreeMode = false
    this.finalHandledInCurrentRun = false
    this.textInsertionInFlight = false
    this.processingCancelable = false
    this.captureReadyPromise = null
    this.finalReceivedAt = 0
    this.pendingHistoryArtifact = null
    this.currentActiveAppContext = null
    this.currentPromptResolution = null
    this.cachedProbeResult = null
    this.consecutiveSilentSamples = 0
    this.consecutiveNonVoicedSamples = 0
    this.quietRunSawSignal = false
    this.consecutiveVoicedSamples = 0
    this.currentVolumeWarning = 'none'
    this.hasDetectedVoiceThisSession = false
    this.lastLowVolumeWarnAt = 0
    this.osMicMuted = false
    this.pendingOsMicMuted = false
    this.pendingOsMicMutedSamples = 0
    clearCapturedInsertionTarget()
    this.recordStartPerf = 0
    if (!options?.preserveLateFinalContext) {
      this.timedOutProcessingContext = null
    }
    this.transition('idle')
    if (!options?.keepOverlay) {
      this.overlayService.clearFallbackHideTimer()
      this.overlayService.hide()
    }
  }

  private async discardCanceledHistory(artifact: { recordId: string; audioFilePath?: string }) {
    try {
      if (artifact.audioFilePath) await bridge.deleteAudioFile(artifact.audioFilePath)
    } catch { /* 取消清理失败不阻塞状态复位 */ }
    try {
      await deleteHistory(artifact.recordId)
      void bridge.emit('history-updated')
    } catch { /* 记录可能尚未写入；processFinalResult 完成写入后会再次清理 */ }
  }

  private async cancelRecording(
    token: number,
    options: { showCanceled?: boolean; reason?: 'user' | 'mic_muted_timeout' } = {},
  ) {
    const showCanceled = options.showCanceled ?? true
    const reason = options.reason ?? 'user'
    if (
      this.state !== 'recording'
      || token === 0
      || token !== this.activeRunId
    ) {
      addRuntimeEvent('info', 'recorder', 'Ignored recording cancellation', {
        state: this.state,
        token,
        activeRunId: this.activeRunId,
        reason,
      })
      return
    }

    const canceledRunId = this.activeRunId
    // 在任何 await 之前离开 recording 并让 run 失效。这样同时到达的 PTT up 不会
    // 再进入正常 stopRecording，Provider 的迟到 partial/error 也会被 run guard 丢弃。
    if (!this.transition('processing')) return
    this.finalHandledInCurrentRun = true
    this.processingCancelable = false
    this.overlayService.stopListeningTicker()
    this.restoreSystemMuteIfNeeded()
    this.recordedChunks = []
    this.finishRun(canceledRunId)
    this.provider.cancel()

    addRuntimeEvent('info', 'recorder', 'Recording canceled', {
      runId: canceledRunId,
      mode: this.provider.mode,
      reason,
    })
    if (showCanceled) this.overlayService.showCanceled()

    try {
      await stopCapture()
    } catch (error) {
      addRuntimeEvent('warn', 'recorder', 'Failed to stop capture while canceling', { error: String(error) })
    } finally {
      // state 在停止采集期间保持 processing，阻止新录音复用同一个 capture；确认旧
      // capture 已结束后再回 idle。keepOverlay 保留短暂的“已取消”反馈。
      if (this.getState() === 'processing' && this.activeRunId === 0) {
        this.resetToIdle({ keepOverlay: showCanceled })
      }
      if (this.provider.mode === 'server') this.ensureConnection()
    }
  }

  private cancelProcessing(token: number) {
    if (
      this.state !== 'processing'
      || token === 0
      || token !== this.activeRunId
      || !this.processingCancelable
    ) {
      addRuntimeEvent('info', 'recorder', 'Ignored Esc processing cancellation', {
        state: this.state,
        token,
        activeRunId: this.activeRunId,
        processingCancelable: this.processingCancelable,
      })
      return
    }

    const canceledRunId = this.activeRunId
    const historyArtifact = this.pendingHistoryArtifact
    this.clearProcessingTimeout()
    this.timedOutProcessingContext = null
    this.processingCancelable = false
    this.provider.cancel()
    this.recordedChunks = []
    this.finishRun(canceledRunId)
    if (historyArtifact?.runId === canceledRunId) {
      void this.discardCanceledHistory(historyArtifact)
    }

    // token 与当前 run 在同一同步检查内通过后才允许提示“已取消”；旧队列动作不会影响新代次。
    addRuntimeEvent('info', 'recorder', 'Processing canceled by user', {
      runId: canceledRunId,
      mode: this.provider.mode,
    })
    this.overlayService.showCanceled()
    this.resetToIdle({ keepOverlay: true })

    // ServerProvider.cancel 会主动断开旧 socket，隔离可能迟到的服务端结果。
    if (this.provider.mode === 'server') this.ensureConnection()
  }

  // ── Provider callbacks ──

  private buildProviderCallbacks(): TranscriptionCallbacks {
    return {
      onPartialASR: (text) => {
        // 仅在录音进行中把流式中间结果推给悬浮窗；下一次 listening 心跳会带上它一起渲染
        if (this.state !== 'recording') return
        this.overlayService.setStreamingText(text)
      },

      onASR: (result) => {
        if (this.state !== 'processing') return
        if (this.finalHandledInCurrentRun) return
        if (result.text && result.text.trim() !== '') return

        const runId = this.activeRunId
        if (!this.isRunCurrent(runId)) return
        // 空 ASR 已是本代终态：同步阻止 timeout 与后续 final 再次收尾。
        this.finalHandledInCurrentRun = true
        this.clearProcessingTimeout()

        const audioDur = this.getAudioDurationSec()
        const wallSec = this.wallTimeAtStopSec > 0 ? this.wallTimeAtStopSec : audioDur
        const audioChunks = this.recordedChunks.slice()
        const historyMeta = this.buildHistoryMetadata(
          this.currentPromptResolution,
          this.currentActiveAppContext,
        )
        void (async () => {
          let historyArtifact: { runId: number; recordId: string; audioFilePath?: string } | null = null
          try {
            const historyEnabled = await getSetting('historyEnabled', true)
            if (!this.isRunCurrent(runId)) return
            if (historyEnabled) {
              const recordId = Date.now().toString(36) + Math.random().toString(36).slice(2, 6)
              historyArtifact = { runId, recordId }
              this.pendingHistoryArtifact = historyArtifact
              const saveAudioEnabled = await getSetting('audioRetentionEnabled', true)
              if (!this.isRunCurrent(runId)) return
              if (saveAudioEnabled && audioChunks.length > 0) {
                try {
                  const savedPath = await saveRecordingAudio(recordId, audioChunks)
                  if (savedPath) historyArtifact.audioFilePath = savedPath
                } catch (err) {
                  addRuntimeEvent('warn', 'recorder', 'Failed to save audio for empty result', { error: String(err) })
                }
                // 保存文件后必须重新验代；失效时同时清文件和可能已存在的记录。
                if (!this.isRunCurrent(runId)) {
                  await this.discardCanceledHistory(historyArtifact)
                  return
                }
              }
              await addHistory({
                id: recordId,
                timestamp: Date.now(),
                asrText: '',
                llmText: '',
                asrMs: result.asrMs || 0,
                llmMs: 0,
                durationSec: wallSec,
                audioDurationSec: audioDur > 0 ? audioDur : undefined,
                asrDurationSec: result.durationSec > 0 ? result.durationSec : undefined,
                charCount: 0,
                isEmpty: true,
                // 调用本身成功，只是没出字。这一句和上面的报错要能区分开：
                // 前者多半真没说话，后者是调用失败
                failReason: t('recorder.noTranscript'),
                failReasonCode: 'no_transcript',
                aiSource: result.aiSource,
                aiStatus: result.aiStatus,
                audioFilePath: historyArtifact.audioFilePath,
                ...historyMeta,
              })
              // 写记录后同样验代；取消/新 run 会清掉刚写入的文件与记录。
              if (!this.isRunCurrent(runId)) {
                await this.discardCanceledHistory(historyArtifact)
                return
              }
              void bridge.emit('history-updated')
            }
          } catch (err) {
            if (historyArtifact) await this.discardCanceledHistory(historyArtifact)
            if (!this.isRunCurrent(runId)) return
            addRuntimeEvent('warn', 'recorder', 'Failed to write empty result to history', { error: String(err) })
          }

          if (!this.isRunCurrent(runId)) return
          this.overlayService.showNoSpeech({
            reason: 'asr_empty',
            mode: this.provider.mode,
            audioSec: Number(audioDur.toFixed(1)),
            asrMs: result.asrMs || 0,
            audioChunks: audioChunks.length,
            runId,
          })
          this.finishRun(runId)
          this.resetToIdle({ keepOverlay: true })
        })()
      },

      onFinal: (result) => {
        if (this.state !== 'processing') {
          const lateContext = this.consumeTimedOutProcessingContext()
          if (!lateContext) return

          addRuntimeEvent('warn', 'recorder', 'Received a late final result for a timed-out session', {
            timedOutAt: lateContext.timedOutAt,
            lateByMs: Date.now() - lateContext.timedOutAt,
            durationSec: result.durationSec,
            asrMs: result.asrMs,
            llmMs: result.llmMs,
          })
          // final 已经快照到前端，立即废弃旧 Provider 会话；尤其 Server 必须断开旧 socket，
          // 防止随后迟到的 done/error 在下一代 start 后误清新会话。
          this.provider.cancel()
          if (this.provider.mode === 'server') this.ensureConnection()
          this.finalizingLateRunId = lateContext.runId
          void this.processFinalResult(result, lateContext, {
            allowInsertionWhenIdle: true,
            source: 'late_after_timeout',
          }).finally(() => {
            if (this.finalizingLateRunId === lateContext.runId) this.finalizingLateRunId = 0
          })
          return
        }
        if (this.finalHandledInCurrentRun) {
          addRuntimeEvent('warn', 'recorder', 'Ignored duplicate final result')
          return
        }
        this.finalHandledInCurrentRun = true
        // 首个 processing final 在同步回调路径立即撤销 timeout，避免二者双收尾。
        this.clearProcessingTimeout()
        this.finalReceivedAt = Date.now()

        const localAudioDur = this.getAudioDurationSec()
        console.log('[ptt-diag] onFinal', {
          backendDurationSec: result.durationSec,
          localAudioDurSec: localAudioDur.toFixed(2),
          audioSentSamples: this.audioSentSamples,
          asrMs: result.asrMs,
          llmMs: result.llmMs,
        })

        void this.processFinalResult(result, {
          runId: this.activeRunId,
          timedOutAt: 0,
          audioDurationSec: this.getAudioDurationSec(),
          wallTimeSec: this.wallTimeAtStopSec > 0 ? this.wallTimeAtStopSec : this.getAudioDurationSec(),
          promptResolution: this.currentPromptResolution ? { ...this.currentPromptResolution } : null,
          appContext: this.currentActiveAppContext ? { ...this.currentActiveAppContext } : null,
          audioChunks: this.recordedChunks.slice(),
          probeResult: this.cachedProbeResult ? { ...this.cachedProbeResult } : null,
        }, {
          allowInsertionWhenIdle: false,
          source: 'processing',
        })
      },

      onDone: () => {
        if (this.state !== 'processing') return
        if (this.finalHandledInCurrentRun) return
        if (this.textInsertionInFlight) return
        const runId = this.activeRunId
        this.clearProcessingTimeout()
        this.finishRun(runId)
        this.resetToIdle()
      },

      onError: (msg) => {
        const runId = this.activeRunId
        if (!this.isRunCurrent(runId) || (this.state !== 'recording' && this.state !== 'processing')) {
          addRuntimeEvent('warn', 'backend', 'Ignored error callback from a stale session', { msg, state: this.state, runId })
          return
        }
        const friendlyFailure = describeProviderError(msg)
        addRuntimeEvent('error', 'backend', msg)
        this.clearProcessingTimeout()
        this.processingCancelable = false
        void this.overlayService.disableEscapeAction()
        this.finalHandledInCurrentRun = true

        // 停止音频采集，并进入不可再触发 stopRecording 的收尾状态。
        if (this.state === 'recording') {
          this.overlayService.stopListeningTicker()
          void stopCapture().catch(() => { })
          this.restoreSystemMuteIfNeeded()
          this.transition('processing')
        }

        // 快照本代音频和元数据；每个异步 artifact 边界后均验代并清理失效产物。
        const audioDur = this.getAudioDurationSec()
        const wallSec = this.wallTimeAtStopSec > 0 ? this.wallTimeAtStopSec : audioDur
        const audioChunks = this.recordedChunks.slice()
        const historyMeta = this.buildHistoryMetadata(
          this.currentPromptResolution,
          this.currentActiveAppContext,
        )
        void (async () => {
          let historyArtifact: { runId: number; recordId: string; audioFilePath?: string } | null = null
          try {
            if (audioDur >= 0.5 && audioChunks.length > 0) {
              const historyEnabled = await getSetting('historyEnabled', true)
              if (!this.isRunCurrent(runId)) return
              if (historyEnabled) {
                const recordId = Date.now().toString(36) + Math.random().toString(36).slice(2, 6)
                historyArtifact = { runId, recordId }
                this.pendingHistoryArtifact = historyArtifact
                const saveAudioEnabled = await getSetting('audioRetentionEnabled', true)
                if (!this.isRunCurrent(runId)) return
                if (saveAudioEnabled) {
                  try {
                    const savedPath = await saveRecordingAudio(recordId, audioChunks)
                    if (savedPath) historyArtifact.audioFilePath = savedPath
                  } catch (err) {
                    addRuntimeEvent('warn', 'recorder', 'Failed to save audio during error recovery', { error: String(err) })
                  }
                  if (!this.isRunCurrent(runId)) {
                    await this.discardCanceledHistory(historyArtifact)
                    return
                  }
                }
                await addHistory({
                  id: recordId,
                  timestamp: Date.now(),
                  asrText: '',
                  llmText: '',
                  asrMs: 0,
                  llmMs: 0,
                  durationSec: wallSec,
                  audioDurationSec: audioDur > 0 ? audioDur : undefined,
                  charCount: 0,
                  isEmpty: true,
                  // 供应商/后端给的原话（额度、资源未开通、连接被断都在这里），
                  // 以前只进日志，用户看到的仍是「无有效声音」
                  failReason: friendlyFailure.detail,
                  failReasonCode: classifyHistoryProviderFailure(msg),
                  audioFilePath: historyArtifact.audioFilePath,
                  ...historyMeta,
                })
                if (!this.isRunCurrent(runId)) {
                  await this.discardCanceledHistory(historyArtifact)
                  return
                }
                void bridge.emit('history-updated')
              }
            }
          } catch (err) {
            if (historyArtifact) await this.discardCanceledHistory(historyArtifact)
            if (!this.isRunCurrent(runId)) return
            addRuntimeEvent('warn', 'recorder', 'Failed to write error history entry', { error: String(err) })
          }

          if (!this.isRunCurrent(runId)) return
          this.finishRun(runId)
          this.resetToIdle()
        })()
      },
    }
  }

  // ── Text insertion: uses pre-probed editable result ──

  private async handleTextInsertion(
    text: string,
    options: { allowWhenIdle?: boolean; runId: number; probeResult?: ProbeResult | null },
  ) {
    const insertionStartedAt = Date.now()
    const { runId } = options
    const allowWhenIdle = options.allowWhenIdle === true
    if (!this.isRunCurrent(runId)) return

    // Guard: if we're no longer in processing (e.g. timeout fired), bail out unless this
    // is the explicitly retained late-final path.
    if (this.state !== 'processing' && !allowWhenIdle) {
      addRuntimeEvent('warn', 'recorder', 'Skipped text insertion because state is no longer processing', { state: this.state, runId })
      return
    }

    // Cancel the processing timeout — we're handling the result now.
    if (this.state === 'processing') {
      this.clearProcessingTimeout()
    }

    // 始终使用本次 final 携带的 probe 快照，不能读取可能已被下一次录音覆盖的实例字段。
    const capturedProbe = options.probeResult ?? null
    const probe = capturedProbe ?? await this.pasteService.getProbeResult()
    if (!this.isRunCurrent(runId)) return

    const usedCapturedProbe = capturedProbe !== null
    const probeAgeMs = typeof probe.completedAt === 'number' ? Date.now() - probe.completedAt : undefined
    const probeDurationMs = (
      typeof probe.completedAt === 'number'
      && typeof probe.startedAt === 'number'
    )
      ? probe.completedAt - probe.startedAt
      : undefined
    addRuntimeEvent('info', 'recorder', 'Paste decision', {
      runId,
      probeId: probe.probeId,
      editable: probe.editable,
      hwnd: probe.hwnd,
      focusHwnd: probe.focusHwnd,
      pid: probe.pid,
      process: probe.process,
      verdict: probe.verdict,
      isCurrentAppProcess: probe.isCurrentAppProcess,
      windowClass: probe.windowClass,
      focusClass: probe.focusClass,
      usedCapturedProbe,
      probeAgeMs,
      probeDurationMs,
      finalToDecisionMs: this.finalReceivedAt > 0 ? insertionStartedAt - this.finalReceivedAt : undefined,
      detail: probe.detail,
      textLen: text.length,
    })

    if (probe.isCurrentAppProcess) {
      // SayIt 自身也是 Chromium 窗口，renderer 直插对 React 受控组件无效
      // （DOM value 被设置但 React state 不同步，下次 re-render 会覆盖）。
      // 所以不走 renderer 直插，而是和外部窗口一样走 Rust paste（Ctrl+V）。
      addRuntimeEvent('info', 'recorder', 'Target is SayIt; using native paste instead of renderer insertion', {
        probeId: probe.probeId,
        editable: probe.editable,
        hwnd: probe.hwnd,
        focusHwnd: probe.focusHwnd,
      })
    }

    if (!probe.editable) {
      // 目标不可编辑时只展示主动操作卡片，不提前改写用户剪贴板。
      addRuntimeEvent('info', 'recorder', 'Target is not editable; showing fallback card', {
        probeId: probe.probeId,
        pid: probe.pid,
        process: probe.process,
        verdict: probe.verdict,
        isCurrentAppProcess: probe.isCurrentAppProcess,
        detail: probe.detail,
      })
      if (!this.isRunCurrent(runId)) return
      this.showFallbackAndReset(text, 'not_editable', runId)
      return
    }

    await this.waitForModifierPTTReleaseIfNeeded()
    if (!this.isRunCurrent(runId)) return

    // pasteText 一旦进入 Rust SendInput 就无法撤销。先关闭本地取消屏障，再确认原生
    // Esc 模式已关闭，最后验代；已排队的旧 cancel 因 processingCancelable=false 只能被忽略。
    this.processingCancelable = false
    await this.overlayService.disableEscapeAction()
    if (!this.isRunCurrent(runId)) return

    // Target was editable → paste (pass the captured probe so Rust uses the original hwnd)
    const pasteStartedAt = Date.now()
    const result = await this.pasteService.pasteText(text, probe, this.cachedProtectClipboard)
    if (!this.isRunCurrent(runId)) return

    if (result.ok) {
      addRuntimeEvent('info', 'recorder', 'External text insertion succeeded', {
        strategy: result.strategy,
        detail: result.detail,
        attempts: result.attempts,
        finalToPasteDoneMs: this.finalReceivedAt > 0 ? Date.now() - this.finalReceivedAt : undefined,
        pasteExecMs: Date.now() - pasteStartedAt,
      })

      this.finishRun(runId)
      if (this.state === 'processing') {
        this.resetToIdle()
      }
      return
    }

    // Paste command failed (SendInput error, timeout, etc.). The card now lets the user
    // explicitly copy or dismiss; do not overwrite the clipboard automatically here.
    const level = result.reason === 'paste_exception' ? 'error' : 'warn'
    addRuntimeEvent(level, 'recorder', 'External text insertion failed; showing fallback card', {
      strategy: result.strategy,
      reason: result.reason,
      detail: result.detail,
      attempts: result.attempts,
      finalToPasteDoneMs: this.finalReceivedAt > 0 ? Date.now() - this.finalReceivedAt : undefined,
      pasteExecMs: Date.now() - pasteStartedAt,
    })
    this.showFallbackAndReset(text, result.reason || 'paste_failed', runId)
  }

  /**
   * Show fallback card and transition to idle.
   * Ensures overlay layout is switched to fallback BEFORE hiding other states.
   */
  private showFallbackAndReset(text: string, reason: string, runId: number) {
    if (!this.isRunCurrent(runId)) return
    addRuntimeEvent('info', 'recorder', 'Showing fallback card', {
      runId,
      reason,
      textLen: text.length,
      stateBeforeReset: this.state,
    })
    // 先发布带本代 token 的 fallback，再结束 run；后续旧 dismiss 只能匹配这张卡片。
    this.activeFallbackToken = runId
    this.overlayService.showFallback(text, reason, runId)
    this.finishRun(runId)
    // Then: transition to idle but keep overlay visible
    if (this.state === 'processing') {
      this.resetToIdle({ keepOverlay: true })
    }
  }

  // ── Connection management ──

  private ensureConnection() {
    if (this.provider.isReady()) return
    this.provider.connect(this.buildProviderCallbacks()).catch((err) => {
      addRuntimeEvent('warn', 'websocket', 'Preconnection failed; retrying in 5s', { error: String(err) })
      setTimeout(() => this.ensureConnection(), 5000)
    })
  }

  // ── Recording lifecycle ──

  private async startRecording() {
    if (
      this.state !== 'idle'
      || this.startRecordingLock
      || this.textInsertionInFlight
      || this.finalizingLateRunId !== 0
    ) {
      addRuntimeEvent('info', 'recorder', 'Ignored start-recording request', {
        state: this.state,
        locked: this.startRecordingLock,
        textInsertionInFlight: this.textInsertionInFlight,
        finalizingLateRunId: this.finalizingLateRunId,
      })
      return
    }

    // 未就绪（如 server 模式后端未连接）：给出告警，不进入录音，避免悬浮窗卡住关不掉
    if (!this.provider.isReady()) {
      this.handsFreeMode = false
      addRuntimeEvent('warn', 'recorder', 'Provider not ready; ignored start-recording request', { mode: this.provider.mode })
      this.overlayService.showError(this.notReadyMessage())
      this.ensureConnection()
      return
    }

    // 新 run 创建前先废弃 timeout 宽限期的旧 provider 代次；后续原有 connect 流程
    // 会为 ServerProvider 建立新 socket，Cloud/Local 也会因 cancel 丢弃旧回调。
    const timedOutContext = this.timedOutProcessingContext
    if (timedOutContext) {
      this.timedOutProcessingContext = null
      this.provider.cancel()
      this.finishRun(timedOutContext.runId)
    }

    const runId = ++this.runSequence
    this.activeRunId = runId
    this.activeFallbackToken = 0
    this.startRecordingLock = true
    this.pendingStopWhileStarting = false
    this.timedOutProcessingContext = null
    this.clearMicMutedAutoCancelTimer()
    this.osMicMuted = false
    this.pendingOsMicMuted = false
    this.pendingOsMicMutedSamples = 0

    // 先给用户即时视觉反馈；overlay 不聚焦，因此后续原生上下文捕获仍指向原目标窗口。
    // 首次 WebView 已在启动空闲期预热，正常情况下这里只剩一次轻量 show + emit。
    this.overlayService.showWaiting()
    // 不等待上下文捕获、Provider 建连或 AudioWorklet 初始化：提前拿到系统静音标志，
    // 好让第一帧 PCM 一到就能立刻裁决（标志本身不足以判定，见 pendingOsMicMuted）。
    if (!this.remoteSession) void this.checkConfiguredMicMuted(runId)

    const targetCapture = captureActiveInsertionTarget(undefined, {
      preserveExistingOnFailure: true,
    })
    let activeAppContext: ActiveAppContext | null = null
    try {
      // AI is the only consumer of editor text. If cleanup is off, do not read the text even when
      // the preference remains enabled, so the privacy boundary matches actual behavior.
      const includeTextContext = this.cachedContextAwareWriting && this.cachedAiEnabled
      const recordingContext = await bridge.getRecordingContext(includeTextContext)
      activeAppContext = recordingContext.appContext as unknown as ActiveAppContext
      this.cachedProbeResult = recordingContext.probe as unknown as ProbeResult
      addRuntimeEvent('info', 'recorder', 'Insertion probe cached at recording start', {
        probeId: this.cachedProbeResult.probeId,
        hwnd: this.cachedProbeResult.hwnd,
        focusHwnd: this.cachedProbeResult.focusHwnd,
        editable: this.cachedProbeResult.editable,
        process: this.cachedProbeResult.process,
        verdict: this.cachedProbeResult.verdict,
      })
    } catch {
      activeAppContext = null
      this.cachedProbeResult = null
    }
    const textContext = usableTextContext(activeAppContext?.textContext)
    if (activeAppContext) {
      if (textContext) activeAppContext.textContext = textContext
      else delete activeAppContext.textContext
    }
    this.currentActiveAppContext = activeAppContext

    this.currentPromptResolution = resolvePromptRouting({
      appContext: activeAppContext,
      presets: this.cachedPresets,
      activePresetId: this.cachedActivePresetId,
      appRules: this.cachedAppPromptRules,
      userStats: this.cachedUserStats,
      hotwords: this.cachedHotwords,
      injectHotwords: this.cachedInjectHotwords,
    })
    if (textContext) {
      this.currentPromptResolution = {
        ...this.currentPromptResolution,
        systemPrompt: withContextAwareInstructions(
          this.currentPromptResolution.systemPrompt,
          textContext,
          this.cachedContextSelectionEditPrompt,
        ),
        summary: `${this.currentPromptResolution.summary} | Text context: ${textContext.selectedText ? 'selection' : 'caret'}`,
      }
      addRuntimeEvent('info', 'recorder', 'Text context captured', {
        source: textContext.source,
        beforeLen: textContext.textBefore.length,
        selectedLen: textContext.selectedText.length,
        afterLen: textContext.textAfter.length,
      })
    }

    addRuntimeEvent('info', 'recorder', 'Recording started', {
      micId: this.remoteSession ? REMOTE_MIC_ID : this.cachedMicId || 'default',
      preset: this.currentPromptResolution.preset.id || this.currentPromptResolution.preset.name || 'none',
      targetCapture,
      appContext: this.summarizeAppContext(activeAppContext || null),
      promptRouting: {
        appId: this.currentPromptResolution.appId,
        appName: this.currentPromptResolution.appName,
        presetId: this.currentPromptResolution.preset.id,
        presetName: this.currentPromptResolution.preset.name,
        promptRuleId: this.currentPromptResolution.matchedRule?.id,
        summary: this.currentPromptResolution.summary,
      },
    })
    addRuntimeEvent('info', 'personalization', 'Prompt routing resolved', {
      appContext: this.summarizeAppContext(activeAppContext || null),
      appId: this.currentPromptResolution.appId,
      appName: this.currentPromptResolution.appName,
      presetId: this.currentPromptResolution.preset.id,
      presetName: this.currentPromptResolution.preset.name,
      promptRuleId: this.currentPromptResolution.matchedRule?.id,
      summary: this.currentPromptResolution.summary,
    })

    this.finalHandledInCurrentRun = false
    this.audioSentSamples = 0
    this.wallTimeAtStopSec = 0
    this.recordedChunks = []
    // Reset audio stats
    this.audioStatsRmsSum = 0
    this.audioStatsPeakRms = 0
    this.audioStatsPeakAmplitude = 0
    this.audioStatsSilentFrames = 0
    this.audioStatsTotalFrames = 0
    this.consecutiveSilentSamples = 0
    this.consecutiveNonVoicedSamples = 0
    this.quietRunSawSignal = false
    this.consecutiveVoicedSamples = 0
    this.currentVolumeWarning = 'none'
    this.hasDetectedVoiceThisSession = false
    this.lastLowVolumeWarnAt = 0
    resetWaveformBarState(this.overlayWaveState, this.overlayService.getBarCount(), 3)

    // Hands-free mode: arm a 5-minute auto-stop timer
    // is handled by the Rust keyboard hook's hard_timeout_release)
    const armHandsFreeTimer = () => {
      if (!this.handsFreeMode) return
      this.handsFreeAutoStopId = setTimeout(() => {
        if (this.state === 'recording' && this.handsFreeMode) {
          addRuntimeEvent('warn', 'recorder', 'Hands-free recording is approaching the five-minute limit')
          this.overlayService.showTimeoutWarning()
          // Auto-stop after 1 more minute
          this.handsFreeAutoStopId = setTimeout(() => {
            if (this.state === 'recording' && this.handsFreeMode) {
              addRuntimeEvent('warn', 'recorder', 'Hands-free recording reached five minutes and stopped automatically')
              void this.stopRecording()
            }
          }, RECORDING_COUNTDOWN_SEC * 1000)
        }
        // 与上限统一由常量推导，避免"改了上限忘了改提醒时机"
      }, (MAX_RECORDING_SEC - RECORDING_COUNTDOWN_SEC) * 1000)
    }

    const promptOpts = this.currentPromptResolution
      ? {
        runId,
        systemPrompt: this.cachedAiEnabled ? this.currentPromptResolution.systemPrompt : undefined,
        disableAi: !this.cachedAiEnabled,
        aiMinDurationSec: this.cachedAiMinDurationSec,
        clientMeta: this.cachedClientRuntimeInfo,
        appContext: activeAppContext,
        textContext,
        hotwords: this.cachedHotwords.length > 0 ? this.cachedHotwords : undefined,
        language: this.cachedLanguage || undefined,
        streamingDisplay: this.cachedStreamingDisplay,
      }
      : {
        runId,
        disableAi: !this.cachedAiEnabled,
        aiMinDurationSec: this.cachedAiMinDurationSec,
        clientMeta: this.cachedClientRuntimeInfo,
        appContext: activeAppContext,
        textContext,
        hotwords: this.cachedHotwords.length > 0 ? this.cachedHotwords : undefined,
        language: this.cachedLanguage || undefined,
        streamingDisplay: this.cachedStreamingDisplay,
      }

    // Wrap the async setup so stopRecording can wait for it
    let resolveCaptureReady: () => void
    this.captureReadyPromise = new Promise<void>((resolve) => { resolveCaptureReady = resolve })

    try {

      // 并行执行 WebSocket 连接和麦克风采集，减少等待时间
      const [, captureResult] = await Promise.all([
        this.provider.connect(this.buildProviderCallbacks()),
        startCapture(
          this.remoteSession ? REMOTE_MIC_ID : this.cachedMicId || undefined,
          (buffer) => {
            if (!this.isRunCurrent(runId)) return
            if (this.audioSentSamples === 0) {
              console.log('[ptt-diag] first onData buffer', {
                byteLength: buffer.byteLength,
                samples: buffer.byteLength / 2,
              })
            }
            this.recordedChunks.push(buffer.slice(0))
            this.provider.sendAudio(buffer)
          },
          undefined,
          (pcmFrame) => {
            if (!this.isRunCurrent(runId)) return
            this.audioSentSamples += pcmFrame.length
            const bars = computeBarsFromPCM(pcmFrame, this.overlayWaveState, {
              barCount: this.overlayService.getBarCount(),
              minHeight: 3,
              maxHeight: 18,
            })
            this.overlayService.pushListeningBars(bars)

            // 计算本帧 RMS 与峰值幅度（均归一化到 0..1）。峰值严格为 0 才表示整帧没有输入；
            // 只要存在任何非零采样，就必须和「未检测到声音」区分开。
            let sum = 0
            let framePeakRaw = 0
            for (let i = 0; i < pcmFrame.length; i++) {
              const s = pcmFrame[i]
              sum += s * s
              const amp = s < 0 ? -s : s
              if (amp > framePeakRaw) framePeakRaw = amp
            }
            const rms = Math.sqrt(sum / pcmFrame.length) / 32768
            const framePeak = framePeakRaw / 32768

            // Audio stats tracking
            this.audioStatsTotalFrames++
            this.audioStatsRmsSum += rms
            if (rms > this.audioStatsPeakRms) this.audioStatsPeakRms = rms
            if (framePeak > this.audioStatsPeakAmplitude) this.audioStatsPeakAmplitude = framePeak
            if (rms < RecorderOrchestrator.SILENCE_RMS_THRESHOLD) this.audioStatsSilentFrames++

            // 两档提醒：静音（未检测到声音）/ 偏低（请靠近麦克风）
            this.updateVolumeWarning(classifyMicLevel(rms, framePeak), pcmFrame.length)
          },
          this.noiseSuppression,
        ),
      ])
      resolveCaptureReady!()
      if (!this.isRunCurrent(runId)) {
        await stopCapture().catch(() => { })
        return
      }

      // Both WebSocket and mic are ready — send start command
      const started = this.provider.start(promptOpts)
      if (!started) {
        throw new Error('sendStart failed')
      }
      addRuntimeEvent('info', 'recorder', 'Start sent; audio capture beginning')

      // Audio capture is now active — transition to recording state and show audio bars
      this.recordStartPerf = performance.now()
      if (!this.transition('recording')) {
        this.startRecordingLock = false
        this.provider.cancel()
        this.finishRun(runId)
        this.resetToIdle()
        if (this.provider.mode === 'server') this.ensureConnection()
        return
      }
      this.startRecordingLock = false
      // 若本次会走流式实时显示，录音一开始就让气泡显示占位，避免中途弹出+缩放导致的抖动。
      // 实时读取供应商/WorkspaceId（避免切换供应商后缓存过期，导致非实时模型也弹气泡）。
      void this.applyStreamingActive()
      this.overlayService.startListeningTicker(runId)
      const micSource = describeMicSource(captureResult, this.remoteSession ? REMOTE_MIC_ID : this.cachedMicId, t('mic.title'))
      if (micSourceChanged(this.lastMicSourceIdentity, micSource.identity)) {
        this.lastMicSourceIdentity = micSource.identity
        this.overlayService.showMicSourceHint({ mode: micSource.mode, label: micSource.label })
        addRuntimeEvent('info', 'recorder', 'Input source reminder shown', {
          mode: micSource.mode,
          label: micSource.label,
        })
      }
      // 录音一开始就查一次麦克风是否被系统静音，被静音则悬浮窗即时红色高警（不阻塞录音）
      // 用 getUserMedia 实际打开的设备再复核；若系统在初始化期间切换了默认麦克风，以这里为准。
      if (!this.remoteSession) void this.checkMicMuted(
        captureResult.label || null,
        runId,
        ++this.micMuteProbeSequence,
      )
      // 就绪提示音已触发，稍后再静音系统输出（避免把提示音一起静掉）
      this.scheduleSystemMuteIfEnabled()
      armHandsFreeTimer()

      // Check if PTT up arrived while we were initializing
      if (this.pendingStopWhileStarting) {
        this.pendingStopWhileStarting = false
        addRuntimeEvent('info', 'recorder', 'PTT released during initialization; stopping immediately')
        void this.stopRecording()
        return
      }

    } catch (error) {
      resolveCaptureReady!()
      this.startRecordingLock = false
      this.pendingStopWhileStarting = false
      // 录音启动失败也要恢复系统输出，避免系统一直静音
      this.restoreSystemMuteIfNeeded()
      addRuntimeEvent('error', 'recorder', 'Failed to start recording', { error: String(error) })
      try { await stopCapture() } catch { /* ignore */ }
      this.finishRun(runId)
      this.provider.cancel()
      // 在悬浮窗显示错误信息，让用户知道发生了什么
      const errMsg = String(error)
      if (errMsg.includes('麦克风') || errMsg.includes('microphone') || errMsg.includes('audio')) { // i18n-allow: 匹配底层中文错误串
        this.overlayService.showError(t('recorder.microphoneUnavailable'))
      } else {
        this.overlayService.showError(t('recorder.startFailed'))
      }
      this.resetToIdle({ keepOverlay: true })
    }
  }

  /**
   * 录音时的音量提醒状态机（两档）：
   *  - muted（PCM 全 0）：麦克风没有任何输入 → 「未检测到声音」
   *  - low （有声音但 RMS 偏低）：距离远 / 说太小声 → 「请靠近麦克风」
   *  - voiced（正常）：连续约 0.5s 即认定恢复，清除提醒
   *
   * 关键准确性约束：「未检测到声音」只在「从录音开始就没听到过正常说话，且这整段安静里
   * 每个采样都为 0」时才报；任何非零输入都只能算声音小。这样也避免把说话后的正常停顿
   * 思考误判成设备故障（那种情况只温和提示「请靠近麦克风」）。
   * 计时/迟滞沿用原逻辑：没听到过说话时更早提醒（2s），
   * 听到过后放宽（5s）；同档每 5s 重弹；连续正常音量 0.5s 才清警告，避免瞬时小声闪烁。
   */
  private updateVolumeWarning(level: MicLevel, sampleCount: number) {
    const REWARN_MS = 5000
    const CLEAR_VOICED = 8000 // ~0.5s @16kHz 累计正常音量才清除
    const VOICED_GAP_TOLERANCE = 4800 // ~300ms：容忍说话中字与字之间的短暂停顿，不清零已累计的清除进度
    const firstWarn = this.hasDetectedVoiceThisSession ? 80000 : 32000 // 5s / 2s @16kHz

    // 系统报了端点静音，但那只是设置标志，不代表采集流真被切断（见 pendingOsMicMuted）。
    // 用真实 PCM 裁决：全 0 累计到阈值才升级成红色高警；一旦出现任何非零采样，
    // 就说明这块设备的 mute 标志不作数，本次录音永久丢弃这条线索。
    if (this.pendingOsMicMuted) {
      const decision = judgeOsMicMute(this.pendingOsMicMutedSamples, level, sampleCount)
      if (decision.verdict === 'confirmed') {
        this.pendingOsMicMuted = false
        this.pendingOsMicMutedSamples = 0
        this.osMicMuted = true
        addRuntimeEvent('warn', 'recorder', 'Microphone is muted by the operating system (confirmed: audio is all-zero)')
        this.overlayService.showMicMutedAlert()
        this.scheduleMicMutedAutoCancel(this.activeRunId)
      } else if (decision.verdict === 'dismissed') {
        this.pendingOsMicMuted = false
        this.pendingOsMicMutedSamples = 0
        addRuntimeEvent('info', 'recorder', 'System reported the microphone as muted but audio is flowing; ignoring the flag')
      } else {
        this.pendingOsMicMutedSamples = decision.silentSamples
      }
    }

    // 系统已确认被静音：保持红色高警，不让琥珀提醒覆盖它；
    // 一旦收到真实说话（用户中途取消了静音）立即恢复正常。
    if (this.osMicMuted) {
      if (level === 'voiced') {
        this.consecutiveVoicedSamples += sampleCount
        this.consecutiveNonVoicedSamples = 0
        if (this.consecutiveVoicedSamples < CLEAR_VOICED) return
      } else {
        this.consecutiveNonVoicedSamples += sampleCount
        if (this.consecutiveNonVoicedSamples >= VOICED_GAP_TOLERANCE) this.consecutiveVoicedSamples = 0
        return
      }
      this.osMicMuted = false
      this.clearMicMutedAutoCancelTimer()
      this.overlayService.clearWarning()
      this.currentVolumeWarning = 'none'
      this.consecutiveVoicedSamples = 0
      this.consecutiveSilentSamples = 0
      this.consecutiveNonVoicedSamples = 0
      this.quietRunSawSignal = false
      this.hasDetectedVoiceThisSession = true
      return
    }

    if (level === 'voiced') {
      // 即使还不足 0.5s、尚未确认恢复正常，也已经证明输入不为零。
      this.quietRunSawSignal = true
      this.consecutiveVoicedSamples += sampleCount
      this.consecutiveNonVoicedSamples = 0
      if (this.consecutiveVoicedSamples >= CLEAR_VOICED) {
        this.hasDetectedVoiceThisSession = true
        this.consecutiveSilentSamples = 0
        this.quietRunSawSignal = false
        if (this.currentVolumeWarning !== 'none') {
          this.overlayService.clearWarning()
          this.currentVolumeWarning = 'none'
        }
      }
      return
    }

    // 非正常音量（muted / low）：累计安静时长；任何非零输入都永久标记本段并非「无声音」。
    this.consecutiveSilentSamples += sampleCount
    this.consecutiveNonVoicedSamples += sampleCount
    if (level === 'low') this.quietRunSawSignal = true
    // 说话时字与字之间的短暂停顿（<~300ms）不清零已累计的正常音量，否则一句话里的自然
    // 停顿会反复把"清除进度"打回 0，导致明明在正常说话也凑不满连续 0.5s、警告始终清不掉。
    // 只有持续安静超过这个小间隙，才认为不是说话停顿、归零重新计。
    if (this.consecutiveNonVoicedSamples >= VOICED_GAP_TOLERANCE) this.consecutiveVoicedSamples = 0

    if (this.consecutiveSilentSamples < firstWarn) return

    // 只有从未正常说过话且整段所有采样均为 0，才允许提示「未检测到声音」。
    const kind: 'muted' | 'low' =
      !this.hasDetectedVoiceThisSession && !this.quietRunSawSignal ? 'muted' : 'low'

    const now = Date.now()
    // 切换到不同档位立即刷新文案；同档位按节流重弹，避免频繁催促
    if (kind === this.currentVolumeWarning && now - this.lastLowVolumeWarnAt < REWARN_MS) return
    this.lastLowVolumeWarnAt = now
    this.currentVolumeWarning = kind
    if (kind === 'muted') {
      addRuntimeEvent('warn', 'recorder', 'No microphone signal detected; device may be wrong or unavailable')
      this.overlayService.showNoSignalWarning()
    } else {
      addRuntimeEvent('warn', 'recorder', 'Microphone volume is low')
      this.overlayService.showLowVolumeWarning()
    }
  }

  /**
   * 录音开始时查询系统麦克风的静音标志，作为「麦克风已被静音」红色高警的**线索**记录下来；
   * 真正弹警告要等音频帧证实 PCM 全 0（见 pendingOsMicMuted）——系统标志会骗人。
   * 仅在能可靠定位设备时才采信（系统默认设备，或按名字唯一匹配到选中设备），
   * 否则（如选了特定设备但拿不到名字/同名多个）返回不判定，交给基于信号的琥珀提醒兜底。
   */
  private async checkMicMuted(
    activeDeviceLabel: string | null,
    runId: number,
    probeSequence: number,
  ) {
    try {
      // 查询本次 getUserMedia 实际打开的端点，而不是根据设置值或 deviceId 猜测。
      // 设备名无法唯一匹配时 Rust 返回 matched=false，继续交给信号检测兜底。
      const res = await invoke<{ matched: boolean; muted: boolean }>('get_mic_mute_state', {
        deviceLabel: activeDeviceLabel,
      })
      // 后发的实际设备复核优先；旧探测、旧录音或已恢复正常说话时都不能再弹。
      if (probeSequence !== this.micMuteProbeSequence || !this.isRunCurrent(runId)) return
      if (this.state !== 'recording' && !(this.state === 'idle' && this.startRecordingLock)) return
      if (this.hasDetectedVoiceThisSession || !res.matched) return

      if (res.muted) {
        // 只记线索、不弹 UI：等音频帧证实真的全 0（原因见 pendingOsMicMuted 注释）。
        // 已确认、或已在等待证实时都不要重置累计进度（预检查与实际设备复核都会走到这里）。
        if (this.osMicMuted || this.pendingOsMicMuted) return
        this.pendingOsMicMuted = true
        this.pendingOsMicMutedSamples = 0
        addRuntimeEvent('info', 'recorder', 'System reports the microphone endpoint is muted; waiting for audio to confirm')
      } else {
        // 实际打开的端点与预检查不一致，或用户在初始化期间取消了静音。
        this.pendingOsMicMuted = false
        this.pendingOsMicMutedSamples = 0
        if (this.osMicMuted) {
          this.osMicMuted = false
          this.clearMicMutedAutoCancelTimer()
          this.overlayService.clearWarning()
        }
      }
    } catch { /* 查询失败不影响录音，交给信号检测兜底 */ }
  }

  private async stopRecording() {
    if (this.state !== 'recording') {
      addRuntimeEvent('info', 'recorder', 'Ignored stop-recording request', { state: this.state })
      return
    }

    const runId = this.activeRunId
    this.clearMicMutedAutoCancelTimer()

    // Wait for capture setup to complete (getUserMedia + AudioWorklet can take time)
    if (this.captureReadyPromise) {
      try {
        await Promise.race([
          this.captureReadyPromise,
          new Promise<void>((resolve) => setTimeout(resolve, 3000)), // 3s max wait
        ])
      } catch { /* ignore */ }
      this.captureReadyPromise = null
    }
    if (this.state !== 'recording' || !this.isRunCurrent(runId)) return

    this.overlayService.stopListeningTicker()
    addRuntimeEvent('info', 'recorder', 'Recording stopped')

    try { await stopCapture() } catch (error) {
      addRuntimeEvent('error', 'recorder', 'Failed to stop audio capture', { error: String(error) })
    }
    // stopCapture 等待期间用户可能按 Esc 使 run 失效；此时绝不能继续 provider.stop。
    if (this.state !== 'recording' || !this.isRunCurrent(runId)) return

    // 采集已停止，恢复系统输出到静音前的状态
    this.restoreSystemMuteIfNeeded()

    const audioDur = this.getAudioDurationSec()
    const pttHoldMs = elapsedSecFromPerf(this.recordStartPerf) * 1000
    const wallTimeSec = pttHoldMs / 1000
    this.wallTimeAtStopSec = wallTimeSec
    console.log('[ptt-diag] stopRecording', {
      audioSentSamples: this.audioSentSamples,
      audioDurSec: audioDur.toFixed(2),
      wallTimeSec: wallTimeSec.toFixed(2),
      durationRatio: pttHoldMs > 0 ? (audioDur / wallTimeSec).toFixed(3) : 'N/A',
    })

    // 数据一致性检查：PCM 时长应接近实际按住时长
    const durationRatio = wallTimeSec > 0 ? audioDur / wallTimeSec : 1
    if (wallTimeSec > 1 && (durationRatio > 2.0 || durationRatio < 0.3)) {
      addRuntimeEvent('warn', 'recorder', 'Unexpected audio byte count; sample rate may not match', {
        audioDurSec: audioDur.toFixed(2),
        wallTimeSec: wallTimeSec.toFixed(2),
        durationRatio: durationRatio.toFixed(3),
        audioSentSamples: this.audioSentSamples,
        recordedChunksCount: this.recordedChunks.length,
        recordedChunksTotalBytes: this.recordedChunks.reduce((s, c) => s + c.byteLength, 0),
      })
      // 不再丢弃音频，保留以便用户回放和重新识别
    }
    if (audioDur < 0.5) {
      addRuntimeEvent('info', 'recorder', 'Recording shorter than 0.5s; canceled provider and discarded audio')
      this.provider.cancel()
      this.finishRun(runId)
      this.resetToIdle()
      if (this.provider.mode === 'server') this.ensureConnection()
      return
    }

    // 短语音门槛只能等到这里才判断：时长要录完才知道。audioDur 用的是边发边累计的
    // 已发送采样数（不是本地攒下来的音频），所以不影响上传，服务器模式照旧流式。
    //
    // 本地/云 API 模式的 AI 是 Provider 在 ASR 之后自己另发的一次请求，它们在那时用
    // 同一个时长自行判断；这里这份只给服务器模式用 —— 服务端的 AI 紧跟 ASR 执行，
    // 客户端插不进中间，只能把结论随 stop 一起送过去。
    const skipAiForShortSpeech = this.cachedAiMinDurationSec > 0
      && audioDur < this.cachedAiMinDurationSec

    this.provider.stop({
      pttHoldMs,
      disableAi: skipAiForShortSpeech || undefined,
      audioStats: this.audioStatsTotalFrames > 0 ? {
        avgRms: Math.round((this.audioStatsRmsSum / this.audioStatsTotalFrames) * 10000) / 10000,
        peakRms: Math.round(this.audioStatsPeakRms * 10000) / 10000,
        peakAmplitude: Math.round(this.audioStatsPeakAmplitude * 10000) / 10000,
        silenceRatio: Math.round((this.audioStatsSilentFrames / this.audioStatsTotalFrames) * 1000) / 1000,
        totalFrames: this.audioStatsTotalFrames,
      } : undefined,
    })
    addRuntimeEvent('info', 'recorder', 'Stop sent', {
      audioSec: audioDur,
      pttHoldMs: Math.round(pttHoldMs),
      aiMinDurationSec: this.cachedAiMinDurationSec || undefined,
      skipAiForShortSpeech: skipAiForShortSpeech || undefined,
    })

    if (!this.transition('processing')) {
      this.finishRun(runId)
      this.resetToIdle()
      return
    }
    this.processingCancelable = true

    const processingTimeoutMs = this.computeProcessingTimeoutMs(audioDur)
    addRuntimeEvent('info', 'recorder', 'Entered processing', {
      audioSec: audioDur,
      timeoutMs: processingTimeoutMs,
    })
    this.overlayService.showThinking(audioDur, runId)
    this.processingTimeoutId = setTimeout(() => {
      if (this.state !== 'processing' || !this.isRunCurrent(runId)) return
      if (this.textInsertionInFlight) {
        addRuntimeEvent('warn', 'recorder', 'Processing timed out while text insertion is active; extending wait')
        return
      }
      const timedOutCtx: TimedOutProcessingContext = {
        runId,
        timedOutAt: Date.now(),
        audioDurationSec: audioDur,
        wallTimeSec,
        promptResolution: this.currentPromptResolution ? { ...this.currentPromptResolution } : null,
        appContext: this.currentActiveAppContext ? { ...this.currentActiveAppContext } : null,
        audioChunks: this.recordedChunks.slice(),
        probeResult: this.cachedProbeResult ? { ...this.cachedProbeResult } : null,
      }
      this.timedOutProcessingContext = timedOutCtx
      addRuntimeEvent('warn', 'recorder', 'Processing timed out; returning to idle', {
        audioSec: audioDur,
        timeoutMs: processingTimeoutMs,
        lateFinalGraceMs: LATE_FINAL_GRACE_MS,
      })

      // 安全网：快照本次录音音频与元数据。若宽限期内没等到迟到的 final，就把音频存入历史
      // （标记为空结果），用户可在历史里“重新识别”，避免超时丢失整段录音。
      // resetToIdle 不会清空 recordedChunks，这里再 slice 一份，防止后续录音替换缓冲。
      const audioChunks = timedOutCtx.audioChunks
      const historyMeta = this.buildHistoryMetadata(timedOutCtx.promptResolution, timedOutCtx.appContext)
      window.setTimeout(() => {
        // onFinal 若在宽限期内消费了 context（迟到 final 已落历史）→ 跳过，避免重复记录。
        if (this.timedOutProcessingContext !== timedOutCtx || !this.isRunCurrent(timedOutCtx.runId)) return
        // 宽限期已结束，不再接受迟到结果。先废弃 Provider 旧会话，再异步保存兜底历史；
        // Server 必须换 socket，避免旧 final/done/error 串到下一代。
        this.timedOutProcessingContext = null
        this.provider.cancel()
        if (this.provider.mode === 'server') this.ensureConnection()
        void (async () => {
          let historyArtifact: { recordId: string; audioFilePath?: string } | null = null
          try {
            const historyEnabled = await getSetting('historyEnabled', true)
            if (!this.isRunCurrent(timedOutCtx.runId)) return
            if (!historyEnabled) {
              this.finishRun(timedOutCtx.runId)
              return
            }

            let audioFilePath: string | undefined
            const recordId = Date.now().toString(36) + Math.random().toString(36).slice(2, 6)
            const saveAudioEnabled = await getSetting('audioRetentionEnabled', true)
            if (!this.isRunCurrent(timedOutCtx.runId)) return
            if (saveAudioEnabled && audioChunks.length > 0) {
              try {
                const savedPath = await saveRecordingAudio(recordId, audioChunks)
                if (savedPath) audioFilePath = savedPath
              } catch (err) {
                addRuntimeEvent('warn', 'recorder', 'Failed to save audio for timeout recovery', { error: String(err) })
              }
            }
            historyArtifact = { recordId, audioFilePath }
            if (!this.isRunCurrent(timedOutCtx.runId)) {
              await this.discardCanceledHistory(historyArtifact)
              return
            }

            await addHistory({
              id: recordId,
              timestamp: Date.now(),
              asrText: '',
              llmText: '',
              asrMs: 0,
              llmMs: 0,
              durationSec: timedOutCtx.wallTimeSec,
              audioDurationSec: timedOutCtx.audioDurationSec > 0 ? timedOutCtx.audioDurationSec : undefined,
              charCount: 0,
              isEmpty: true,
              audioFilePath,
              ...historyMeta,
            })
            if (!this.isRunCurrent(timedOutCtx.runId)) {
              await this.discardCanceledHistory(historyArtifact)
              return
            }
            void bridge.emit('history-updated')
            this.finishRun(timedOutCtx.runId)
            addRuntimeEvent('info', 'recorder', 'Timed-out recording saved to history for retry', { audioSec: timedOutCtx.audioDurationSec })
          } catch (err) {
            if (historyArtifact) await this.discardCanceledHistory(historyArtifact)
            if (!this.isRunCurrent(timedOutCtx.runId)) return
            this.finishRun(timedOutCtx.runId)
            addRuntimeEvent('warn', 'recorder', 'Failed to write timeout recovery history entry', { error: String(err) })
          }
        })()
      }, LATE_FINAL_GRACE_MS)

      this.resetToIdle({ preserveLateFinalContext: true })
    }, processingTimeoutMs)
  }

  // ── Toggle / hands-free ──

  private pttToggle(isHandsFree = false) {
    const now = Date.now()
    if (now - this.lastToggleTime < 500) {
      addRuntimeEvent('info', 'ptt', 'Ignored toggle request', {
        isHandsFree,
        ignoreReason: 'cooldown',
        recorderState: this.state,
      })
      return
    }
    this.lastToggleTime = now

    if (this.state === 'idle') {
      if (isHandsFree) {
        this.handsFreeMode = true
        this.pttSuppressed = true
        setTimeout(() => { this.pttSuppressed = false }, 500)
      }
      addRuntimeEvent('info', 'ptt', 'toggle -> startRecording', {
        isHandsFree,
        recorderState: this.state,
      })
      void this.startRecording()
      return
    }

    if (this.state === 'recording') {
      if (isHandsFree || !this.handsFreeMode) {
        this.handsFreeMode = false
        addRuntimeEvent('info', 'ptt', 'toggle -> stopRecording', {
          isHandsFree,
          recorderState: this.state,
        })
        void this.stopRecording()
      }
      return
    }

    addRuntimeEvent('info', 'ptt', 'Ignored toggle request', {
      isHandsFree,
      ignoreReason: 'state_not_toggleable',
      recorderState: this.state,
      handsFreeMode: this.handsFreeMode,
    })
  }

  private getPTTEventContext(payload?: unknown) {
    const p = (payload && typeof payload === 'object')
      ? (payload as PTTEventPayload)
      : {}
    return {
      source: p.source || 'unknown',
      keycode: p.keycode,
      rawcode: p.rawcode,
      modifiers: {
        alt: p.altKey,
        ctrl: p.ctrlKey,
        shift: p.shiftKey,
      },
      reason: p.reason,
      pttSetting: p.pttSetting,
      timestamp: p.timestamp,
      recorderState: this.state,
      handsFreeMode: this.handsFreeMode,
      pttSuppressed: this.pttSuppressed,
    }
  }

  private logPTTEvent(event: 'down' | 'up' | 'toggle' | 'hands_free', payload?: unknown) {
    addRuntimeEvent('info', 'ptt', `event:${event}`, this.getPTTEventContext(payload))
  }

  private notePTTDown(payload?: unknown) {
    const p = (payload && typeof payload === 'object')
      ? (payload as PTTEventPayload)
      : {}
    this.lastPTTUpUsedModifier = Boolean(
      p.altKey
      || p.ctrlKey
      || p.shiftKey
      || this.isModifierPTTSetting(p.pttSetting),
    )
  }

  private notePTTUp(payload?: unknown) {
    const p = (payload && typeof payload === 'object')
      ? (payload as PTTEventPayload)
      : {}
    this.lastPTTUpAt = Date.now()
    this.lastPTTUpUsedModifier = Boolean(
      p.altKey
      || p.ctrlKey
      || p.shiftKey
      || this.isModifierPTTSetting(p.pttSetting),
    )
  }

  private isModifierPTTSetting(pttSetting?: string) {
    return _isModifierPTTSetting(pttSetting)
  }

  private async waitForModifierPTTReleaseIfNeeded() {
    if (!this.lastPTTUpUsedModifier || this.lastPTTUpAt <= 0) return
    const elapsedMs = Date.now() - this.lastPTTUpAt
    if (elapsedMs >= MODIFIER_PTT_RELEASE_GUARD_MS) return
    const waitMs = MODIFIER_PTT_RELEASE_GUARD_MS - elapsedMs
    addRuntimeEvent('info', 'recorder', 'Waiting for modifier keys to settle before inserting text', {
      waitMs,
      lastPTTUpAt: this.lastPTTUpAt,
    })
    await new Promise((resolve) => setTimeout(resolve, waitMs))
  }

  private summarizeAppContext(context: ActiveAppContext | null) {
    return _summarizeAppContext(context)
  }

  private buildStatsAppId(appContext: ActiveAppContext | null, promptResolution: PromptResolution | null) {
    return _buildStatsAppId(appContext, promptResolution?.appId)
  }

  private buildHistoryMetadata(
    promptResolution?: PromptResolution | null,
    appContext?: ActiveAppContext | null,
  ) {
    const resolved = promptResolution || this.currentPromptResolution || undefined
    const ctx = appContext === undefined ? this.currentActiveAppContext : appContext
    return {
      appId: resolved?.appId,
      appName: resolved?.appName,
      // 录音时聚焦窗口的原始信息（用于反馈排错）
      windowTitle: ctx?.windowTitle || undefined,
      processName: ctx?.processName || undefined,
      windowClass: ctx?.windowClass || undefined,
      promptPresetId: resolved?.preset.id,
      promptPresetName: resolved?.preset.name,
      promptRuleId: resolved?.matchedRule?.id,
      promptSummary: resolved?.summary,
      workMode: this.provider.mode,
    }
  }

  /** 异步获取当前模式下的 ASR/AI 供应商信息 */
  private async buildProviderMetadata(finalResult?: FinalResult): Promise<{
    asrProvider?: string
    aiProvider?: string
    aiModel?: string
    aiSource?: FinalResult['aiSource']
    aiStatus?: FinalResult['aiStatus']
  }> {
    const mode = this.provider.mode
    const executionMeta = {
      aiSource: finalResult?.aiSource,
      aiStatus: finalResult?.aiStatus,
    }
    if (mode === 'server') {
      let asrProvider = finalResult?.asrModel || finalResult?.asrEngine || 'server'
      // 后端返回 HuggingFace repo 全名如 "Qwen/Qwen3-ASR-1.7B"，只取模型名
      const slashIdx = asrProvider.lastIndexOf('/')
      if (slashIdx >= 0) asrProvider = asrProvider.slice(slashIdx + 1)
      if (finalResult?.aiSource === 'custom') {
        return {
          asrProvider,
          aiProvider: finalResult.aiProvider,
          aiModel: finalResult.aiModel,
          ...executionMeta,
        }
      }
      return {
        asrProvider,
        aiProvider: finalResult?.aiSource === 'none' ? undefined : 'server',
        ...executionMeta,
      }
    }
    if (mode === 'cloud_api') {
      const asrProviderKey = await getSetting('cloudAsr.provider', '') as string
      // 映射内部 key 到实际模型 ID
      const ASR_MODEL_ID_MAP: Record<string, string> = {
        doubao_v2: 'Doubao-Seed-ASR-2.0',
        qwen: 'qwen3-asr-flash',
        mimo: 'mimo-v2.5-asr',
        groq_whisper: 'whisper-large-v3-turbo',
        qwen_omni_35_plus: 'qwen3.5-omni-plus-realtime',
        qwen_omni_35_flash: 'qwen3.5-omni-flash-realtime',
        qwen_omni_flash: 'qwen3-omni-flash-realtime',
        qwen_omni_turbo: 'qwen-omni-turbo-realtime',
        qwen_omni_plus: 'qwen3.5-omni-plus-realtime',
      }
      const asrProvider = ASR_MODEL_ID_MAP[asrProviderKey] || asrProviderKey || 'cloud'
      const aiProvider = finalResult?.aiProvider || await getSetting('cloudAi.provider', '') as string
      const aiModel = finalResult?.aiModel || await getSetting('cloudAi.model', '') as string
      return { asrProvider, aiProvider: aiProvider || undefined, aiModel: aiModel || undefined, ...executionMeta }
    }
    if (mode === 'local') {
      const modelId = await getSetting('localAsr.modelId', '') as string
      const aiEnabled = Boolean(await getSetting('aiEnabled', false))
      const aiProvider = finalResult?.aiProvider
        || (aiEnabled ? await getSetting('cloudAi.provider', '') as string : undefined)
      const aiModel = finalResult?.aiModel
        || (aiEnabled ? await getSetting('cloudAi.model', '') as string : undefined)
      return { asrProvider: modelId || 'local', aiProvider, aiModel: aiModel || undefined, ...executionMeta }
    }
    return executionMeta
  }

  private computeProcessingTimeoutMs(audioDurationSec: number) {
    return _computeProcessingTimeoutMs(audioDurationSec, this.provider.mode)
  }

  private consumeTimedOutProcessingContext() {
    const context = this.timedOutProcessingContext
    if (!context) return null
    if (!this.isRunCurrent(context.runId)) {
      this.timedOutProcessingContext = null
      return null
    }
    // 超过 15 秒时把 context 留给已排队的宽限期兜底定时器，避免双方都放弃收尾。
    if (Date.now() - context.timedOutAt > LATE_FINAL_GRACE_MS) return null
    this.timedOutProcessingContext = null
    return context
  }

  private async processFinalResult(
    result: FinalResult,
    context: TimedOutProcessingContext,
    options: { allowInsertionWhenIdle: boolean; source: 'processing' | 'late_after_timeout' },
  ) {
    const runId = context.runId
    if (!this.isRunCurrent(runId)) return

    // llmText === asrText 说明这段没经过 AI 整理（极速模式，或后端未跑 LLM 直接回 asrText）。
    // 此时文本仍是纯 ASR，「格式规范」由我们兜底；AI 整理过的文本则把格式交给 AI，只做文本替换。
    // New clients may talk to an older server that does not understand text_context. In that case
    // contextApplied is absent: paste the original selection back unchanged instead of replacing it
    // with a spoken edit command.
    const { baseText, rawAsr, selectedEditWasApplied } = resolveContextAwareOutput({
      asrText: result.asrText,
      llmText: result.llmText,
      contextApplied: result.contextApplied,
      textContext: context.appContext?.textContext,
    })
    let textToPaste: string
    try {
      textToPaste = selectedEditWasApplied
        ? await applyTextTransforms(baseText, { rawAsr })
        : baseText
    } catch (error) {
      if (!this.isRunCurrent(runId)) return
      addRuntimeEvent('warn', 'recorder', 'Text post-processing failed', { error: String(error), runId })
      if (baseText && baseText.trim()) {
        this.showFallbackAndReset(baseText, 'text_transform_failed', runId)
      } else {
        if (this.state === 'processing') {
          this.overlayService.showNoSpeech({
            reason: 'text_transform_failed',
            mode: this.provider.mode,
            error: String(error),
            runId,
          })
        }
        this.finishRun(runId)
        if (this.state === 'processing') this.resetToIdle({ keepOverlay: true })
      }
      return
    }
    if (!this.isRunCurrent(runId)) return

    const hasText = Boolean(textToPaste && textToPaste.trim())
    const audioDur = context.audioDurationSec
    const wallSec = context.wallTimeSec > 0 ? context.wallTimeSec : audioDur
    const promptResolution = context.promptResolution
    const appContext = context.appContext
    let historyArtifact: { runId: number; recordId: string; audioFilePath?: string } | null = null

    addRuntimeEvent('info', 'recorder', 'Final result received', {
      runId,
      hasText,
      asrMs: result.asrMs,
      llmMs: result.llmMs,
      durationSec: result.durationSec,
      audioSec: audioDur,
      textLen: textToPaste ? textToPaste.length : 0,
      source: options.source,
      contextApplied: result.contextApplied,
    })

    try {
      const historyEnabled = await getSetting('historyEnabled', true)
      if (!this.isRunCurrent(runId)) return
      if (historyEnabled) {
        const recordId = Date.now().toString(36) + Math.random().toString(36).slice(2, 6)
        historyArtifact = { runId, recordId }
        const saveAudioEnabled = await getSetting('audioRetentionEnabled', true)
        if (!this.isRunCurrent(runId)) return
        if (saveAudioEnabled && context.audioChunks.length > 0) {
          try {
            const savedPath = await saveRecordingAudio(recordId, context.audioChunks)
            if (savedPath) historyArtifact.audioFilePath = savedPath
          } catch (err) {
            addRuntimeEvent('warn', 'recorder', 'Failed to save audio file', { error: String(err) })
          }
        }

        if (!this.isRunCurrent(runId)) {
          await this.discardCanceledHistory(historyArtifact)
          return
        }
        const providerMeta = await this.buildProviderMetadata(result)
        if (!this.isRunCurrent(runId)) {
          await this.discardCanceledHistory(historyArtifact)
          return
        }

        this.pendingHistoryArtifact = historyArtifact
        await addHistory({
          id: recordId,
          timestamp: Date.now(),
          asrText: result.asrText,
          llmText: textToPaste,
          asrMs: result.asrMs,
          llmMs: result.llmMs,
          durationSec: wallSec,
          audioDurationSec: audioDur > 0 ? audioDur : undefined,
          asrDurationSec: result.durationSec > 0 ? result.durationSec : undefined,
          charCount: hasText ? textToPaste.length : 0,
          isEmpty: !hasText,
          // 区分「ASR 本来就没出字」和「ASR 出了字但后处理把它清空了」——
          // 后者是我们自己的问题（替换规则/格式化），别让用户以为是没识别到
          failReason: hasText
            ? undefined
            : result.asrText?.trim()
              ? t('recorder.emptyAfterProcessing')
              : t('recorder.noTranscript'),
          failReasonCode: hasText
            ? undefined
            : result.asrText?.trim()
              ? 'empty_after_processing'
              : 'no_transcript',
          audioFilePath: historyArtifact.audioFilePath,
          ...this.buildHistoryMetadata(promptResolution, appContext),
          ...providerMeta,
        })
        if (!this.isRunCurrent(runId)) {
          await this.discardCanceledHistory(historyArtifact)
          return
        }
        void bridge.emit('history-updated')
      }
    } catch (error) {
      if (historyArtifact) await this.discardCanceledHistory(historyArtifact)
      if (!this.isRunCurrent(runId)) return
      addRuntimeEvent('warn', 'recorder', 'Failed to write history entry', { error: String(error) })
    }

    if (!this.isRunCurrent(runId)) {
      if (historyArtifact) await this.discardCanceledHistory(historyArtifact)
      return
    }

    if (!hasText) {
      if (this.state === 'processing') {
        // asrLen / llmLen 只记长度不记内容：能区分「ASR 就没出字」和
        // 「ASR 有字但后处理把它清空了」，这两种成因完全不同
        this.overlayService.showNoSpeech({
          reason: 'final_empty',
          mode: this.provider.mode,
          audioSec: Number(audioDur.toFixed(1)),
          asrMs: result.asrMs,
          llmMs: result.llmMs,
          asrLen: result.asrText?.length ?? 0,
          llmLen: result.llmText?.length ?? 0,
          runId,
        })
      }
      this.finishRun(runId)
      if (this.state === 'processing') {
        this.resetToIdle({ keepOverlay: true })
      }
      return
    }

    void this.updatePersonalizationFromFinal(runId, textToPaste, promptResolution, appContext)

    this.textInsertionInFlight = true
    try {
      await this.handleTextInsertion(textToPaste, {
        allowWhenIdle: options.allowInsertionWhenIdle,
        runId,
        probeResult: context.probeResult,
      })
    } catch (error) {
      if (!this.isRunCurrent(runId)) return
      addRuntimeEvent('error', 'recorder', 'Text insertion threw; showing fallback card', { error: String(error), runId })
      this.showFallbackAndReset(textToPaste, 'paste_exception', runId)
    } finally {
      this.textInsertionInFlight = false
    }
  }

  private async updatePersonalizationFromFinal(
    runId: number,
    finalText: string,
    promptResolution: PromptResolution | null,
    appContext: ActiveAppContext | null,
  ) {
    if (!finalText.trim() || !this.isRunCurrent(runId)) return

    try {
      const wordCount = finalText.length
      const appId = this.buildStatsAppId(appContext, promptResolution)

      const nextStats = await recordSessionStats(appId, wordCount)
      if (!this.isRunCurrent(runId)) return
      this.cachedUserStats = nextStats
      addRuntimeEvent('info', 'personalization', 'session stats recorded', {
        appId,
        appName: promptResolution?.appName,
        wordCount,
        totalWords: this.cachedUserStats.totalWords,
        totalSessions: this.cachedUserStats.totalSessions,
      })
    } catch (error) {
      if (!this.isRunCurrent(runId)) return
      addRuntimeEvent('warn', 'personalization', 'failed to record session stats', {
        error: String(error),
      })
    }
  }
}
