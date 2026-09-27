// Audio capture service — AudioWorklet with ScriptProcessorNode fallback.
// WebView2 on Windows has known issues where AudioWorklet's process() never
// fires despite addModule() succeeding.  We detect this and fall back to the
// deprecated-but-reliable ScriptProcessorNode.

import { addRuntimeEvent } from './debugLog'
import { REMOTE_MIC_ID, openRemoteCapture, closeRemoteCapture } from './remoteCapture'

let audioCtx: AudioContext | null = null
let workletNode: AudioWorkletNode | null = null
let scriptNode: ScriptProcessorNode | null = null
let mediaStream: MediaStream | null = null
let sourceNode: MediaStreamAudioSourceNode | null = null
let onAudioData: ((buffer: ArrayBuffer) => void) | null = null
let onVolumeChange: ((volume: number) => void) | null = null
let onPCMFrame: ((pcm: Int16Array) => void) | null = null
let actualSampleRate = 16000
let firstPCMFrameLogged = false
let firstRmsLogged = false
let usingFallback = false

const TARGET_SAMPLE_RATE = 16000

/**
 * 一个输入端点。只留判断"这是哪个麦克风"用得到的三个字段，不用 MediaDeviceInfo
 * 本身 —— 那是带方法的宿主对象，单测里构造起来啰嗦。
 */
export interface MicEndpoint {
  deviceId: string
  groupId: string
  label: string
}

/**
 * Chromium 在真实端点之外还摆两个「伪设备」：deviceId 固定为 default /
 * communications，label 是 `<本地化前缀> - <真实端点名>`（中文系统是「默认值 - 」）。
 * 它们不是麦克风，只是「跟着系统默认走」这条路由。
 *
 * 放在这里 export 而不是放在 micSourceReminder 里：设置页的设备列表和悬浮窗的
 * 来源提示都要认它，而 audio.ts 不能反向 import recorder 下的模块（会成环）。
 */
export function isPseudoInputDevice(deviceId: string): boolean {
  const id = deviceId.trim().toLowerCase()
  return !id || id === 'default' || id === 'communications'
}

/**
 * 规范化设置里存的「选中的麦克风」。
 *
 * 伪设备 id 一律折成空串，也就是「跟随系统默认」—— 两者语义完全一样（`getUserMedia`
 * 收到 `deviceId: 'default'` 和收到不带 deviceId 的约束，解析结果是同一个端点）。
 *
 * 必须折：伪设备已经不在 listMicrophones 的结果里了，留着会让设置页的下拉找不到
 * 选中项、退回占位符文案，看起来像"没有选择麦克风"。实测存量数据里确实有
 * `selectedMic = "default"` —— 老版本的下拉把伪设备也列出来，用户点了它。
 */
export function normalizeSelectedMicId(raw: unknown): string {
  const id = typeof raw === 'string' ? raw.trim() : ''
  return isPseudoInputDevice(id) ? '' : id
}

/**
 * 剥掉设备名结尾的 USB 标识（`(047f:c053)` 这种 VID:PID）。
 *
 * 它对辨认设备毫无帮助：同一型号的两台设备 VID:PID 完全一样，连"区分同型号"都做不到，
 * 只是白占宽度、把真正有用的型号名挤出可视范围。设备名里剩下的两段都要留 ——
 * 主名（`耳机式麦克风`）说明是什么，括号里的型号（`Plantronics Blackwire 5220 Series`）
 * 才是区分设备的依据。
 */
export function stripUsbIds(label: string): string {
  return label.replace(/\s*\([0-9a-f]{4}:[0-9a-f]{4}\)\s*$/i, '').trim()
}

/** 列表里真正的麦克风：排掉伪设备，也排掉读不到名字的条目（没名字无从辨认）。 */
export function realInputEndpoints<T extends MicEndpoint>(devices: T[]): T[] {
  return devices.filter((d) => !isPseudoInputDevice(d.deviceId) && d.label.trim().length > 0)
}

/**
 * 把一条「跟随系统默认」的伪设备落到它当下实际指向的那个真实端点。
 *
 * 两级判据，都与界面语言无关（**不要退化成按「默认值 - 」这类文案做匹配**，那等于
 * 给每种界面语言维护一份，Windows 改写法还会静默失效）：
 *  1. groupId 相同 —— 伪设备与它指向的真实端点同组；
 *  2. 退一步按后缀匹配 —— 伪设备的 label 就是「前缀 + 真实 label」，所以真实 label
 *     一定是它的后缀。有些机器上伪设备的 groupId 是空的，只能靠这一层。
 *
 * 两个调用方：设置页用它回答「系统默认现在是哪个设备」，悬浮窗的来源提示用它把
 * 带前缀的名字换成真名。判据只此一份，别再各写一遍。
 */
export function matchRealEndpoint<T extends MicEndpoint>(
  hint: MicEndpoint,
  devices: T[],
): T | null {
  const pool = realInputEndpoints(devices)
  if (pool.length === 0) return null

  if (!isPseudoInputDevice(hint.deviceId)) {
    const exact = hint.deviceId.trim()
    return pool.find((d) => d.deviceId.trim() === exact) ?? null
  }

  const groupId = hint.groupId.trim()
  if (groupId) {
    const byGroup = pool.find((d) => d.groupId.trim() === groupId)
    if (byGroup) return byGroup
  }

  const label = hint.label.trim()
  if (!label) return null
  return pool.find((d) => {
    const candidate = d.label.trim()
    return candidate.length > 0 && label.endsWith(candidate)
  }) ?? null
}

/** The microphone endpoint that getUserMedia actually opened. */
export interface ActiveMicrophoneInfo {
  deviceId: string
  groupId: string
  label: string
  /**
   * 与本次采集同一时刻的输入端点快照。
   *
   * 为什么要带上它：`label` 是 Chromium 给的，"跟随系统默认"这条路由上它是
   * `默认值 - 端点名 (父设备名)`，两头都是噪音。要把它收成人能看懂的一段，需要
   * 知道同一时刻还有哪些端点（见 micSourceReminder.describeMicSource）。
   *
   * 必须在流打开之后枚举：没有活跃流、或没有持久麦克风权限时，
   * `enumerateDevices()` 返回的条目 label 全是空的（见 listMicrophones 的注释），
   * 那种快照对同名判断毫无用处。
   */
  devices: MicEndpoint[]
}

// HMR cleanup: tear down audio capture when module is hot-replaced
if ((import.meta as unknown as Record<string, unknown>).hot) {
  const hot = (import.meta as unknown as Record<string, unknown>).hot as { dispose: (cb: () => void) => void }
  hot.dispose(() => {
    console.log('[audio] HMR dispose: tearing down audio capture')
    if (workletNode) {
      try { workletNode.port.onmessage = null } catch { /* ignore */ }
      try { workletNode.disconnect() } catch { /* ignore */ }
      workletNode = null
    }
    if (scriptNode) {
      try { scriptNode.onaudioprocess = null } catch { /* ignore */ }
      try { scriptNode.disconnect() } catch { /* ignore */ }
      scriptNode = null
    }
    if (sourceNode) {
      try { sourceNode.disconnect() } catch { /* ignore */ }
      sourceNode = null
    }
    if (mediaStream) {
      mediaStream.getTracks().forEach((track) => track.stop())
      mediaStream = null
    }
    if (audioCtx && audioCtx.state !== 'closed') {
      audioCtx.close().catch(() => { })
    }
    audioCtx = null
    onAudioData = null
    onVolumeChange = null
    onPCMFrame = null
  })
}

export function getActualSampleRate(): number {
  return actualSampleRate
}

const PCM_WORKLET_CODE = `
class PCMProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const opts = options.processorOptions || {};
    this.targetRate = opts.targetRate || 16000;
    this.inputRate = opts.inputRate || sampleRate || 48000;
    this.ratio = this.inputRate / this.targetRate;
    this.tail = new Float32Array(0);
    this.phase = 0;
  }

  process(inputs) {
    const input = inputs[0]?.[0];
    if (!input || input.length === 0) {
      return true;
    }

    let sum = 0;
    for (let i = 0; i < input.length; i++) {
      const s = Math.max(-1, Math.min(1, input[i]));
      sum += s * s;
    }
    const rms = Math.sqrt(sum / input.length);

    const merged = new Float32Array(this.tail.length + input.length);
    merged.set(this.tail, 0);
    merged.set(input, this.tail.length);

    let outFloat;

    if (Math.abs(this.ratio - 1) < 0.0001) {
      outFloat = merged;
      this.tail = new Float32Array(0);
      this.phase = 0;
    } else {
      const available = merged.length - this.phase;
      const outLen = Math.floor(available / this.ratio);

      if (outLen <= 0) {
        this.tail = merged;
        this.port.postMessage({ rms, sampleRate: this.targetRate });
        return true;
      }

      outFloat = new Float32Array(outLen);
      for (let i = 0; i < outLen; i++) {
        const pos = this.phase + i * this.ratio;
        const i0 = Math.floor(pos);
        const i1 = Math.min(i0 + 1, merged.length - 1);
        const frac = pos - i0;
        outFloat[i] = merged[i0] * (1 - frac) + merged[i1] * frac;
      }

      const consumed = this.phase + outLen * this.ratio;
      const keepFrom = Math.floor(consumed);
      this.phase = consumed - keepFrom;
      this.tail = keepFrom < merged.length ? merged.slice(keepFrom) : new Float32Array(0);
    }

    const int16 = new Int16Array(outFloat.length);
    for (let i = 0; i < outFloat.length; i++) {
      const s = Math.max(-1, Math.min(1, outFloat[i]));
      int16[i] = s < 0 ? s * 32768 : s * 32767;
    }

    this.port.postMessage({ pcm: int16.buffer, rms, sampleRate: this.targetRate }, [int16.buffer]);
    return true;
  }
}

registerProcessor('pcm-processor', PCMProcessor);
`

/**
 * 输入端点快照。**只在有活跃流时调用才有意义**（见 ActiveMicrophoneInfo.devices）。
 *
 * 与 listMicrophones 的区别：这里绝不为了拿名字去开临时流 —— 它跑在录音启动路径上，
 * 那时候已经有一路采集在跑，再开一路是真实故障。失败一律退化成空快照，
 * 调用方会落到"只显示端点名"这条兜底，不影响采集。
 */
async function snapshotInputEndpoints(): Promise<MicEndpoint[]> {
  try {
    const list = await navigator.mediaDevices.enumerateDevices()
    return list
      .filter((d) => d.kind === 'audioinput')
      .map((d) => ({
        deviceId: String(d.deviceId || ''),
        groupId: String(d.groupId || ''),
        label: String(d.label || ''),
      }))
  } catch {
    return []
  }
}

export async function listMicrophones(): Promise<MediaDeviceInfo[]> {
  const audioInputs = (list: MediaDeviceInfo[]) => list.filter((d) => d.kind === 'audioinput')

  let devices = audioInputs(await navigator.mediaDevices.enumerateDevices())

  // 没有持久麦克风权限时，enumerateDevices() 只返回一个无名字的通用项。
  // 此时临时打开一路音频流触发授权，趁流活跃时重新枚举，拿到真实名称。
  if (devices.length <= 1 || devices.some((d) => !d.label)) {
    let stream: MediaStream | null = null
    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: true })
      devices = audioInputs(await navigator.mediaDevices.enumerateDevices())
    } catch {
      // 权限被拒或无可用麦克风：保留已有结果
    } finally {
      if (stream) stream.getTracks().forEach((t) => t.stop())
    }
  }

  // **原样返回，含 Chromium 的伪设备**。
  //
  // 这里刻意不替调用方剔除伪设备：它们虽然不是麦克风，却携带唯一一条「系统默认现在
  // 指向谁」的信息（label 是 `<前缀> - <真实端点名>`，groupId 指向那个端点）。设置页
  // 要靠它把第一项写成「系统默认（某某麦克风）」。
  // 「哪些该进下拉」是产品判断，归 buildMicOptions；这个函数只负责"系统报了什么"。
  return devices
}

function createAudioContext() {
  const AudioContextCtor = window.AudioContext || (window as Window & { webkitAudioContext?: typeof AudioContext }).webkitAudioContext
  if (!AudioContextCtor) {
    throw new Error('Current browser does not support AudioContext')
  }
  // 优先直接开 16 kHz 的 AudioContext：拿到的话浏览器会用自己的高质量重采样器，
  // 我们下面那套手写线性插值就可以整段跳过。线性插值没有抗混叠低通，48k→16k
  // 是 3 倍抽取，8~24 kHz 的能量会折回 0~8 kHz，直接损害识别准确度。
  // 不是所有 WebView2/浏览器都接受 sampleRate 约束，所以拿不到就回退原路径。
  for (const opts of [
    { latencyHint: 'interactive', sampleRate: TARGET_SAMPLE_RATE, sinkId: { type: 'none' } },
    { latencyHint: 'interactive', sampleRate: TARGET_SAMPLE_RATE },
    { latencyHint: 'interactive', sinkId: { type: 'none' } },
    { latencyHint: 'interactive' },
  ] as unknown as AudioContextOptions[]) {
    try {
      return new AudioContextCtor(opts)
    } catch {
      // 该组合不被支持，试下一个
    }
  }
  return new AudioContextCtor()
}

async function teardownCapture() {
  if (workletNode) {
    try { workletNode.port.onmessage = null } catch { /* ignore */ }
    try { workletNode.disconnect() } catch { /* ignore */ }
    workletNode = null
  }

  if (scriptNode) {
    try { scriptNode.onaudioprocess = null } catch { /* ignore */ }
    try { scriptNode.disconnect() } catch { /* ignore */ }
    scriptNode = null
  }

  if (sourceNode) {
    try { sourceNode.disconnect() } catch { /* ignore */ }
    sourceNode = null
  }

  if (mediaStream) {
    mediaStream.getTracks().forEach((track) => track.stop())
    mediaStream = null
  }

  if (audioCtx && audioCtx.state !== 'closed') {
    await audioCtx.close().catch(() => { })
  }
  audioCtx = null
  actualSampleRate = TARGET_SAMPLE_RATE
}

// ── Resampler state for ScriptProcessorNode fallback ──
let spnTail = new Float32Array(0)
let spnPhase = 0

/** Resample + convert to Int16 (same algorithm as the AudioWorklet) */
function resampleToInt16(input: Float32Array, inputRate: number): { pcm: Int16Array; rms: number } {
  const ratio = inputRate / TARGET_SAMPLE_RATE

  let sum = 0
  for (let i = 0; i < input.length; i++) {
    const s = Math.max(-1, Math.min(1, input[i]))
    sum += s * s
  }
  const rms = Math.sqrt(sum / input.length)

  const merged = new Float32Array(spnTail.length + input.length)
  merged.set(spnTail, 0)
  merged.set(input, spnTail.length)

  let outFloat: Float32Array

  if (Math.abs(ratio - 1) < 0.0001) {
    outFloat = merged
    spnTail = new Float32Array(0)
    spnPhase = 0
  } else {
    const available = merged.length - spnPhase
    const outLen = Math.floor(available / ratio)

    if (outLen <= 0) {
      spnTail = merged
      return { pcm: new Int16Array(0), rms }
    }

    outFloat = new Float32Array(outLen)
    for (let i = 0; i < outLen; i++) {
      const pos = spnPhase + i * ratio
      const i0 = Math.floor(pos)
      const i1 = Math.min(i0 + 1, merged.length - 1)
      const frac = pos - i0
      outFloat[i] = merged[i0] * (1 - frac) + merged[i1] * frac
    }

    const consumed = spnPhase + outLen * ratio
    const keepFrom = Math.floor(consumed)
    spnPhase = consumed - keepFrom
    spnTail = keepFrom < merged.length ? merged.slice(keepFrom) : new Float32Array(0)
  }

  const int16 = new Int16Array(outFloat.length)
  for (let i = 0; i < outFloat.length; i++) {
    const s = Math.max(-1, Math.min(1, outFloat[i]))
    int16[i] = s < 0 ? s * 32768 : s * 32767
  }

  return { pcm: int16, rms }
}

/** Wire up ScriptProcessorNode as fallback when AudioWorklet fails */
function setupScriptProcessorFallback(ctx: AudioContext, src: MediaStreamAudioSourceNode) {
  usingFallback = true
  spnTail = new Float32Array(0)
  spnPhase = 0
  actualSampleRate = TARGET_SAMPLE_RATE

  // Disconnect worklet if it was connected
  if (workletNode) {
    try { workletNode.port.onmessage = null } catch { /* ignore */ }
    try { workletNode.disconnect() } catch { /* ignore */ }
    try { src.disconnect(workletNode) } catch { /* ignore */ }
    workletNode = null
  }

  // 4096 samples buffer — good balance between latency and efficiency
  const spn = ctx.createScriptProcessor(4096, 1, 1)
  scriptNode = spn

  let totalPCMBytes = 0
  let totalPCMFrames = 0
  let captureStartTime = performance.now()

  spn.onaudioprocess = (event) => {
    const input = event.inputBuffer.getChannelData(0)
    const { pcm, rms } = resampleToInt16(input, ctx.sampleRate)

    // 刻意不加 `rms > 0` 这个条件（worklet 路径也没有）：rms 一直是 0 恰恰是要抓的现象，
    // 「有声音才记」等于把最需要的那条证据丢掉 —— 于是日志里「麦克风全程无声」和
    // 「这条日志我压根没实现」长得一模一样。
    if (!firstRmsLogged) {
      firstRmsLogged = true
      addRuntimeEvent('info', 'audio', 'First RMS received (ScriptProcessor)', {
        rms: Number(rms.toFixed(6)),
        sampleRate: TARGET_SAMPLE_RATE,
      })
    }
    onVolumeChange?.(rms)

    if (pcm.length > 0) {
      const pcmBuffer = pcm.buffer as ArrayBuffer
      totalPCMBytes += pcmBuffer.byteLength
      totalPCMFrames++

      if (!firstPCMFrameLogged) {
        firstPCMFrameLogged = true
        captureStartTime = performance.now()
        let peak = 0
        for (let i = 0; i < pcm.length; i++) {
          const value = Math.abs(pcm[i])
          if (value > peak) peak = value
        }
        addRuntimeEvent('info', 'audio', 'First PCM frame received (ScriptProcessor)', {
          samples: pcm.length,
          peak,
          byteLength: pcmBuffer.byteLength,
          sampleRate: TARGET_SAMPLE_RATE,
        })
        console.log('[audio-diag] first PCM frame (ScriptProcessor)', {
          samples: pcm.length,
          byteLength: pcmBuffer.byteLength,
          contextSampleRate: ctx.sampleRate,
          targetSampleRate: TARGET_SAMPLE_RATE,
        })
      }

      if (totalPCMFrames % 100 === 0) {
        const elapsedSec = (performance.now() - captureStartTime) / 1000
        const pcmDurationSec = (totalPCMBytes / 2) / 16000
        console.log('[audio-diag] PCM total (ScriptProcessor)', {
          frames: totalPCMFrames,
          totalBytes: totalPCMBytes,
          wallTimeSec: elapsedSec.toFixed(2),
          pcmDurationSec: pcmDurationSec.toFixed(2),
        })
      }

      onPCMFrame?.(pcm)
      onAudioData?.(pcmBuffer)
    }
  }

  src.connect(spn)
  // ScriptProcessorNode requires an output connection to work
  spn.connect(ctx.destination)
  console.log('[audio-diag] ScriptProcessorNode fallback active')
  addRuntimeEvent('info', 'audio', 'ScriptProcessorNode fallback activated', {
    bufferSize: 4096,
    inputSampleRate: ctx.sampleRate,
    targetSampleRate: TARGET_SAMPLE_RATE,
  })
}

/** 尝试加载 AudioWorklet 模块（data URL → Blob URL 两种方式） */
async function tryLoadAudioWorklet(ctx: AudioContext): Promise<boolean> {
  try {
    const dataUrl = 'data:application/javascript;base64,' + btoa(PCM_WORKLET_CODE)
    await Promise.race([
      ctx.audioWorklet.addModule(dataUrl),
      new Promise<void>((_, reject) => setTimeout(() => reject(new Error('addModule timeout')), 3000)),
    ])
    console.log('[audio-diag] AudioWorklet module loaded (data URL)')
    return true
  } catch (dataUrlErr) {
    console.warn('[audio-diag] data URL addModule failed:', dataUrlErr)
  }

  try {
    const blob = new Blob([PCM_WORKLET_CODE], { type: 'application/javascript' })
    const blobUrl = URL.createObjectURL(blob)
    try {
      await Promise.race([
        ctx.audioWorklet.addModule(blobUrl),
        new Promise<void>((_, reject) => setTimeout(() => reject(new Error('addModule timeout')), 3000)),
      ])
      console.log('[audio-diag] AudioWorklet module loaded (Blob URL)')
      return true
    } finally {
      URL.revokeObjectURL(blobUrl)
    }
  } catch (blobErr) {
    console.warn('[audio-diag] Blob URL addModule also failed:', blobErr)
  }

  return false
}

/** 创建 AudioWorkletNode 并绑定 onmessage 处理 */
function setupAudioWorkletNode(ctx: AudioContext, src: MediaStreamAudioSourceNode): AudioWorkletNode {
  const node = new AudioWorkletNode(ctx, 'pcm-processor', {
    numberOfInputs: 1,
    numberOfOutputs: 0,
    processorOptions: {
      targetRate: TARGET_SAMPLE_RATE,
      inputRate: ctx.sampleRate,
    },
  })

  addRuntimeEvent('info', 'audio', 'AudioContext ready', {
    contextState: ctx.state,
    inputSampleRate: ctx.sampleRate,
    targetSampleRate: TARGET_SAMPLE_RATE,
    ratio: (ctx.sampleRate / TARGET_SAMPLE_RATE).toFixed(4),
  })

  let totalPCMBytes = 0
  let totalPCMFrames = 0
  let captureStartTime = performance.now()

  node.port.onmessage = (e) => {
    // 标记已收到 worklet 数据（用于外部静默检测）
    node.dispatchEvent(new Event('worklet-data'))

    if (typeof e.data.sampleRate === 'number') {
      actualSampleRate = e.data.sampleRate
    }

    if (typeof e.data.rms === 'number') {
      if (!firstRmsLogged) {
        firstRmsLogged = true
        addRuntimeEvent('info', 'audio', 'First RMS received', {
          rms: Number(e.data.rms.toFixed(6)),
          sampleRate: actualSampleRate,
        })
      }
      onVolumeChange?.(e.data.rms)
    }

    const pcmBuffer = e.data.pcm as ArrayBuffer | undefined
    if (pcmBuffer && pcmBuffer.byteLength > 0) {
      const pcmFrame = new Int16Array(pcmBuffer)
      totalPCMBytes += pcmBuffer.byteLength
      totalPCMFrames++
      if (!firstPCMFrameLogged) {
        firstPCMFrameLogged = true
        captureStartTime = performance.now()
        let peak = 0
        for (let i = 0; i < pcmFrame.length; i++) {
          const value = Math.abs(pcmFrame[i])
          if (value > peak) peak = value
        }
        addRuntimeEvent('info', 'audio', 'First PCM frame received', {
          samples: pcmFrame.length,
          peak,
          byteLength: pcmBuffer.byteLength,
          sampleRate: actualSampleRate,
        })
        console.log('[audio-diag] first PCM frame', {
          samples: pcmFrame.length,
          byteLength: pcmBuffer.byteLength,
          contextSampleRate: audioCtx?.sampleRate,
          targetSampleRate: TARGET_SAMPLE_RATE,
          actualSampleRate,
        })
      }
      if (totalPCMFrames % 5000 === 0) {
        const elapsedSec = (performance.now() - captureStartTime) / 1000
        const pcmDurationSec = (totalPCMBytes / 2) / 16000
        console.log('[audio-diag] PCM total', {
          frames: totalPCMFrames,
          totalBytes: totalPCMBytes,
          wallTimeSec: elapsedSec.toFixed(2),
          pcmDurationSec: pcmDurationSec.toFixed(2),
        })
      }
      onPCMFrame?.(pcmFrame)
      onAudioData?.(pcmBuffer)
    }
  }

  src.connect(node)
  console.log('[audio-diag] sourceNode connected to workletNode')
  return node
}

export async function startCapture(
  deviceId: string | undefined,
  onData: (buffer: ArrayBuffer) => void,
  onVolume?: (volume: number) => void,
  onFrame?: (pcm: Int16Array) => void,
  noiseSuppression: boolean = true,
) {
  // VoiceHub：遥控器直传 PCM（蓝牙音频经宿主 RemoteTransport 到这里），不走系统麦克风。
  if (deviceId === REMOTE_MIC_ID) return openRemoteCapture(onData, onFrame)
  // Always tear down previous capture to prevent stale state leaks
  const hadPriorCtx = audioCtx !== null
  const hadPriorWorklet = workletNode !== null
  const hadPriorStream = mediaStream !== null
  if (hadPriorCtx || hadPriorWorklet || hadPriorStream) {
    console.warn('[audio-diag] startCapture found stale state; cleaning up', {
      hadAudioCtx: hadPriorCtx,
      priorCtxState: audioCtx?.state,
      hadWorklet: hadPriorWorklet,
      hadStream: hadPriorStream,
    })
    addRuntimeEvent('warn', 'audio', 'startCapture cleaned up stale state', {
      hadAudioCtx: hadPriorCtx,
      priorCtxState: audioCtx?.state,
      hadWorklet: hadPriorWorklet,
      hadStream: hadPriorStream,
    })
  }
  await teardownCapture()

  onAudioData = onData
  onVolumeChange = onVolume ?? null
  onPCMFrame = onFrame ?? null
  firstPCMFrameLogged = false
  firstRmsLogged = false
  actualSampleRate = TARGET_SAMPLE_RATE
  usingFallback = false

  const constraints: MediaStreamConstraints = {
    audio: {
      channelCount: 1,
      echoCancellation: false,
      // 默认开启降噪：部分机器麦克风底噪大（低频电流声），关闭降噪会原样录入。
      // 但浏览器降噪对 ASR 是个变量——它按"人听得舒服"优化，可能削掉模型要的细节，
      // 所以做成开关，麦克风环境干净的用户可以关掉对比。
      // 回声消除/自动增益保持关闭，避免影响 ASR 音频。
      noiseSuppression,
      autoGainControl: false,
      ...(deviceId ? { deviceId: { exact: deviceId } } : {}),
    },
  }

  try {
    console.log('[audio-diag] getUserMedia starting...', { deviceId: deviceId || 'default' })
    mediaStream = await navigator.mediaDevices.getUserMedia(constraints)
    const track = mediaStream.getAudioTracks()[0] || null
    const settings = track?.getSettings?.()
    const activeMicrophone: ActiveMicrophoneInfo = {
      // Prefer the resolved track setting over the requested id. In system-default mode
      // these may differ, and the resolved value is what the reminder needs to describe.
      deviceId: String(settings?.deviceId || deviceId || ''),
      groupId: String(settings?.groupId || ''),
      label: String(track?.label || ''),
      // 流已经开着，所以这一次枚举拿得到带名字的条目。多花的是一次进程间调用，
      // 与同一路径上的 getUserMedia / AudioContext 相比可以忽略。
      devices: await snapshotInputEndpoints(),
    }

    console.log('[audio-diag] getUserMedia success', {
      trackCount: mediaStream.getAudioTracks().length,
      trackLabel: track?.label,
      trackEnabled: track?.enabled,
      trackMuted: track?.muted,
      trackReadyState: track?.readyState,
      settings,
    })

    // muted / readyState / enabled 必须落盘：它们区分「Chromium 自己知道这路采集被静音了」
    // 与「Chromium 以为一切正常、实际送来的全是 0」。后者是 WebView2 的已知毛病
    // （采集静默降级成静音且不派发 mute 事件），只能靠对照这三个值 + 后面的 peakAmplitude 判定。
    addRuntimeEvent('info', 'audio', 'Microphone capture started', {
      requestedDeviceId: deviceId || 'default',
      trackLabel: track?.label || '',
      trackMuted: track?.muted ?? null,
      trackEnabled: track?.enabled ?? null,
      trackReadyState: track?.readyState || null,
      trackSettings: settings || null,
    })

    audioCtx = createAudioContext()
    actualSampleRate = audioCtx.sampleRate || TARGET_SAMPLE_RATE
    const nativeSixteenK = audioCtx.sampleRate === TARGET_SAMPLE_RATE
    console.log('[audio-diag] AudioContext created', {
      state: audioCtx.state,
      sampleRate: audioCtx.sampleRate,
      nativeSixteenK,
    })
    // 记一条到运行时日志：这是判断"有没有走上高质量重采样"的唯一依据，
    // 排查识别准确度问题时先看这里。
    addRuntimeEvent('info', 'audio', nativeSixteenK ? 'AudioContext is natively 16 kHz; resampling skipped' : 'AudioContext is not 16 kHz; using linear resampling', {
      contextState: audioCtx.state,
      contextSampleRate: audioCtx.sampleRate,
      targetSampleRate: TARGET_SAMPLE_RATE,
    })

    sourceNode = audioCtx.createMediaStreamSource(mediaStream)

    // Try AudioWorklet, with ScriptProcessor fallback
    const fallbackCtx = audioCtx
    const fallbackSrc = sourceNode
    const fallbackTimerId = setTimeout(() => {
      if (!usingFallback && fallbackCtx === audioCtx && fallbackSrc === sourceNode) {
        console.warn('[audio-diag] AudioWorklet timed out (1.5s); switching to ScriptProcessorNode')
        addRuntimeEvent('warn', 'audio', 'AudioWorklet timed out; switching to ScriptProcessorNode fallback')
        setupScriptProcessorFallback(fallbackCtx, fallbackSrc)
      }
    }, 1500)

    const workletLoaded = await tryLoadAudioWorklet(audioCtx)

    if (audioCtx.state === 'suspended') {
      console.log('[audio-diag] AudioContext is suspended, resuming...')
      await audioCtx.resume()
      console.log('[audio-diag] AudioContext resumed, state:', audioCtx.state)
    }

    if (!workletLoaded) {
      clearTimeout(fallbackTimerId)
      if (!usingFallback) {
        console.log('[audio-diag] AudioWorklet unavailable, using ScriptProcessorNode directly')
        setupScriptProcessorFallback(audioCtx, sourceNode)
      }
      return activeMicrophone
    }

    clearTimeout(fallbackTimerId)
    if (usingFallback) {
      console.log('[audio-diag] ScriptProcessorNode already active, skipping worklet setup')
      return activeMicrophone
    }

    // AudioWorklet loaded — set up node and monitor for data
    workletNode = setupAudioWorkletNode(audioCtx, sourceNode)

    // Secondary monitor: if worklet loaded but no data within 800ms, switch
    let gotWorkletData = false
    workletNode.addEventListener('worklet-data', () => { gotWorkletData = true }, { once: true })
    setTimeout(() => {
      if (!gotWorkletData && !usingFallback && fallbackCtx === audioCtx && fallbackSrc === sourceNode) {
        console.warn('[audio-diag] AudioWorklet produced no data (800ms); switching to ScriptProcessorNode')
        addRuntimeEvent('warn', 'audio', 'AudioWorklet was silent; switching to ScriptProcessorNode fallback')
        setupScriptProcessorFallback(fallbackCtx, fallbackSrc)
      }
    }, 800)

    return activeMicrophone

  } catch (error) {
    await teardownCapture()
    addRuntimeEvent('error', 'audio', 'Failed to start microphone capture', {
      requestedDeviceId: deviceId || 'default',
      error: String(error),
    })
    throw error
  }
}

export async function stopCapture() {
  // VoiceHub：遥控器直传通道有自己的关停路径（RemoteTransport ended/error）。
  closeRemoteCapture()
  const finalCtxRate = audioCtx?.sampleRate
  console.log('[audio-diag] stopCapture final summary', {
    contextSampleRate: finalCtxRate,
    actualSampleRate,
    targetSampleRate: TARGET_SAMPLE_RATE,
    usingFallback,
  })
  addRuntimeEvent('info', 'audio', 'Capture stop summary', {
    contextSampleRate: finalCtxRate,
    actualSampleRate,
    targetSampleRate: TARGET_SAMPLE_RATE,
    usingFallback,
    // 「一帧 PCM 都没到过」需要一条肯定式证据。只靠「日志里没有 First PCM frame received」
    // 来推，读日志的人无法区分它与「这条日志没被镜像」。
    receivedPcm: firstPCMFrameLogged,
  })

  await teardownCapture()

  onAudioData = null
  onVolumeChange = null
  onPCMFrame = null
}
