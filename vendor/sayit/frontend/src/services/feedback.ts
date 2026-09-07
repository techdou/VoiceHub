// 反馈意见服务 — 发送用户反馈到后端

import { getBackendBaseUrl } from './runtimeConfig'
import { getClientRuntimeInfo } from './bridge'
import { getSetting, getActivePreset, listHistory, type HistoryRecord } from './store'
import { getWorkMode } from './transcription'
import { t } from '@/i18n'

export interface FeedbackPayload {
  machine_id: string
  app_version: string
  timestamp: string
  feedback_text: string
  // 转录 + 该次会话的排错信息（仅在用户保留转录时发送）
  transcript: {
    asr_text: string
    ai_text: string
    duration_sec: number
    // 会话排错信息
    app_name?: string
    app_id?: string
    window_title?: string
    process_name?: string
    window_class?: string
    asr_ms?: number
    llm_ms?: number
    audio_duration_sec?: number
    char_count?: number
    is_empty?: boolean
    asr_provider?: string
    ai_provider?: string
    ai_model?: string
    prompt_summary?: string
    auto_applied_hotwords?: string[]
    record_timestamp?: number
  } | null
  context: {
    work_mode: string
    asr_provider: string
    asr_model: string
    ai_enabled: boolean
    ai_provider: string
    ai_model: string
    ai_preset_name: string
    ai_system_prompt: string
    ai_prompt_append: string
    // 客户端环境（非个人信息，用于排错；不含 IP）
    platform: string
    os_version: string
    system_locale: string
    cpu_cores: number
    memory_mb: number
  }
}

export interface FeedbackResult {
  ok: boolean
  message: string
}

/** 获取最后一条转录记录 */
export async function getLastTranscript(): Promise<HistoryRecord | null> {
  const records = await listHistory({ limit: 1, offset: 0 })
  return records.length > 0 ? records[0] : null
}

/** 收集当前配置上下文 */
async function collectContext(
  clientInfo: Awaited<ReturnType<typeof getClientRuntimeInfo>>,
): Promise<FeedbackPayload['context']> {
  const workMode = getWorkMode()
  const aiEnabled = await getSetting('aiEnabled', false) as boolean
  const aiProvider = await getSetting('cloudAi.provider', '') as string
  // 运行时生效的模型只认扁平键。原来这里读的是 `cloudAi.<provider>.model`，那是
  // AI 服务页自己的存储命名空间；页面改成服务列表后那个键不再更新，会报出过期的模型名。
  const aiModel = await getSetting('cloudAi.model', '') as string
  const activePreset = await getActivePreset()
  const aiPromptAppend = await getSetting('aiPromptAppend', '') as string

  // ASR 信息
  let asrProvider = ''
  let asrModel = ''
  if (workMode === 'cloud_api') {
    asrProvider = await getSetting('cloudAsr.provider', '') as string
    asrModel = await getSetting(`cloudAsr.${asrProvider}.model`, '') as string
  } else if (workMode === 'local') {
    asrProvider = 'local'
    asrModel = await getSetting('localAsr.model', '') as string
  } else {
    asrProvider = 'server'
    asrModel = ''
  }

  return {
    work_mode: workMode,
    asr_provider: asrProvider,
    asr_model: asrModel,
    ai_enabled: aiEnabled,
    ai_provider: aiProvider,
    ai_model: aiModel,
    ai_preset_name: activePreset.name,
    ai_system_prompt: activePreset.systemPrompt.slice(0, 5000),
    ai_prompt_append: (aiPromptAppend || '').slice(0, 5000),
    platform: clientInfo.platform || '',
    os_version: clientInfo.osVersion || '',
    system_locale: clientInfo.systemLocale || '',
    cpu_cores: clientInfo.cpuCores || 0,
    memory_mb: clientInfo.memoryMb || 0,
  }
}

/** 发送反馈到后端 */
export async function submitFeedback(feedbackText: string, options?: { includeTranscript?: boolean }): Promise<FeedbackResult> {
  const includeTranscript = options?.includeTranscript ?? true

  // 校验
  const trimmed = feedbackText.trim()
  if (trimmed.length < 2) {
    return { ok: false, message: t('feedback.tooShort') }
  }
  if (trimmed.length > 1000) {
    return { ok: false, message: t('feedback.tooLong') }
  }

  // 收集信息
  const clientInfo = await getClientRuntimeInfo()
  const lastRecord = includeTranscript ? await getLastTranscript() : null
  const context = await collectContext(clientInfo)

  const payload: FeedbackPayload = {
    machine_id: clientInfo.deviceId,
    app_version: clientInfo.clientVersion,
    timestamp: new Date().toISOString(),
    feedback_text: trimmed,
    transcript: lastRecord
      ? {
        asr_text: (lastRecord.asrText || '').slice(0, 5000),
        ai_text: (lastRecord.llmText || '').slice(0, 5000),
        duration_sec: lastRecord.durationSec || 0,
        // 该次会话的排错信息
        app_name: lastRecord.appName || undefined,
        app_id: lastRecord.appId || undefined,
        window_title: lastRecord.windowTitle || undefined,
        process_name: lastRecord.processName || undefined,
        window_class: lastRecord.windowClass || undefined,
        asr_ms: lastRecord.asrMs || undefined,
        llm_ms: lastRecord.llmMs || undefined,
        audio_duration_sec: lastRecord.audioDurationSec || undefined,
        char_count: lastRecord.charCount || undefined,
        is_empty: lastRecord.isEmpty || undefined,
        asr_provider: lastRecord.asrProvider || undefined,
        ai_provider: lastRecord.aiProvider || undefined,
        ai_model: lastRecord.aiModel || undefined,
        prompt_summary: lastRecord.promptSummary || undefined,
        auto_applied_hotwords:
          lastRecord.autoAppliedHotwords && lastRecord.autoAppliedHotwords.length > 0
            ? lastRecord.autoAppliedHotwords
            : undefined,
        record_timestamp: lastRecord.timestamp || undefined,
      }
      : null,
    context,
  }

  // 发送
  const baseUrl = getBackendBaseUrl()
  const res = await fetch(`${baseUrl}/api/feedback`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  })

  if (res.status === 429) {
    return { ok: false, message: t('feedback.tooFrequent') }
  }

  if (!res.ok) {
    return { ok: false, message: t('feedback.sendFailed', { status: res.status }) }
  }

  return { ok: true, message: t('feedback.sent') }
}
