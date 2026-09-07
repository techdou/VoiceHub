/**
 * ASR 供应商 key → 实际模型 ID 映射
 */

const QWEN_OMNI_MODEL_MAP: Record<string, string> = {
  qwen_omni_plus: 'qwen3.5-omni-plus-realtime',
  qwen_omni_35_plus: 'qwen3.5-omni-plus-realtime',
  qwen_omni_35_flash: 'qwen3.5-omni-flash-realtime',
  qwen_omni_flash: 'qwen3-omni-flash-realtime',
  qwen_omni_turbo: 'qwen-omni-turbo-realtime',
}

/** 判断是否为 Qwen Omni 系列模型 */
export function isQwenOmniProvider(provider: string): boolean {
  return provider.startsWith('qwen_omni')
}

/** 根据供应商 key 解析 Qwen Omni 模型 ID，非 Omni 返回 undefined */
export function resolveQwenOmniModel(provider: string): string | undefined {
  return QWEN_OMNI_MODEL_MAP[provider]
}

/** ASR 供应商 key → 显示用的模型 ID */
const ASR_DISPLAY_MODEL_MAP: Record<string, string> = {
  doubao_v2: 'Doubao-Seed-ASR-2.0',
  qwen: 'qwen3-asr-flash',
  qwen_realtime: 'qwen3-asr-flash-realtime',
  qwen_audio_stream: 'qwen-audio-3.0-asr-flash-streaming',
  mimo: 'mimo-v2.5-asr',
  groq_whisper: 'whisper-large-v3-turbo',
  ...QWEN_OMNI_MODEL_MAP,
}

/** 将内部供应商 key 映射为显示用的模型名称 */
export function resolveAsrDisplayModel(providerKey: string): string {
  return ASR_DISPLAY_MODEL_MAP[providerKey] || providerKey || 'unknown'
}

/** 判断某个 ASR 供应商是否为「可能支持」流式实时显示的模型（不含运行时前置条件）。
 *  注意：qwen3-asr-flash（qwen）是非实时 HTTP 模型，不支持；千问这边只有
 *  qwen3-asr-flash-realtime 与 qwen-audio-3.0-asr-flash-streaming 支持。 */
export function isStreamingDisplayCapable(provider: string): boolean {
  return provider === 'doubao_v2'
    || provider === 'qwen_realtime'
    || provider === 'qwen_audio_stream'
}

/**
 * 判断当前配置下「流式实时显示」是否真正就绪可用（含运行时前置条件）。
 * - doubao_v2：直接可用（需在火山开通「流式语音识别 2.0」，运行时由服务端校验）。
 * - qwen_audio_stream（qwen-audio-3.0-asr-flash-streaming）：直接可用。这个模型
 *   **不需要 WorkspaceId** —— 通用域名 dashscope.aliyuncs.com 实测可用，
 *   别照抄下面那条的前置条件（见 providers/asr_qwen_audio_stream.rs 文件头的实测记录）。
 * - qwen_realtime（qwen3-asr-flash-realtime）：走地域专属实时端点，必须提供北京业务空间 WorkspaceId 才可用。
 * - qwen（qwen3-asr-flash）：非实时模型，不支持实时字幕。
 */
export function isStreamingDisplayReady(provider: string, qwenWorkspaceId?: string): boolean {
  if (provider === 'doubao_v2') return true
  if (provider === 'qwen_audio_stream') return true
  if (provider === 'qwen_realtime') {
    return Boolean(qwenWorkspaceId && qwenWorkspaceId.trim())
  }
  return false
}
