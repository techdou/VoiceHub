import { useMemo, useState } from 'react'
import { ChevronDown, ChevronUp } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { useT } from '@/i18n/useT'
import { type DebugSession } from '@/services/debugLog'
import { AudioPlayer, LogItem } from './DebugCards'
import {
  extractPrompts,
  formatDuration,
  formatTime,
  getASRPayload,
  getFinalPayload,
  getHoldDurationSec,
  getReadyPayload,
  getSessionDurationBytes,
  resolveSessionSampleRate,
  USEFUL_LOG_TYPES,
} from './debugUtils'

export default function SessionCard({ session, defaultOpen }: { session: DebugSession; defaultOpen?: boolean }) {
  const t = useT()
  const [open, setOpen] = useState(defaultOpen ?? false)
  const [showAllLogs, setShowAllLogs] = useState(false)

  const final = getFinalPayload(session)
  const asr = getASRPayload(session)
  const ready = getReadyPayload(session)

  const totalBytes = getSessionDurationBytes(session)
  const sampleRate = resolveSessionSampleRate(session)
  const audioDurationSec = Math.floor(totalBytes / 2) / sampleRate
  const holdDurationSec = getHoldDurationSec(session)
  const asrDurationSec = Number(asr?.duration_sec || final?.asr_debug?.duration_sec || 0)

  const prompts = useMemo(() => extractPrompts(session), [session])
  const logs = useMemo(() => {
    if (showAllLogs) return session.messages
    return session.messages.filter((message) => USEFUL_LOG_TYPES.has(message.type))
  }, [session.messages, showAllLogs])

  const filteredOutCount = Math.max(0, session.messages.length - logs.length)

  return (
    <Card>
      <CardContent className="space-y-3 p-4">
        <div className="flex cursor-pointer items-start justify-between gap-2" onClick={() => setOpen((value) => !value)}>
          <div className="min-w-0 flex-1">
            <div className="mb-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
              <span>{formatTime(session.startTime)}</span>
              <span>{t('debug.audioSent', { duration: formatDuration(audioDurationSec) })}</span>
              <span>{t('debug.logCount', { count: session.messages.length })}</span>
              <span>{t('debug.sampleRate', { rate: sampleRate })}</span>
            </div>
            <p className="truncate text-sm text-foreground">
              {(final?.llm_text || final?.asr_text || asr?.text || t('debug.noResult')) as string}
            </p>
          </div>

          <div className="ml-2 flex shrink-0 items-center gap-2">
            <AudioPlayer session={session} />
            {open ? <ChevronUp className="h-4 w-4 text-muted-foreground" /> : <ChevronDown className="h-4 w-4 text-muted-foreground" />}
          </div>
        </div>

        {open && (
          <>
            <div className="grid grid-cols-2 gap-2 text-xs md:grid-cols-5">
              <div className="rounded border bg-muted p-2"><div className="text-muted-foreground">{t('debug.asrTime')}</div><div className="font-medium">{Number(final?.asr_ms || asr?.asr_ms || 0)} ms</div></div>
              <div className="rounded border bg-muted p-2"><div className="text-muted-foreground">{t('debug.llmTime')}</div><div className="font-medium">{Number(final?.llm_ms || 0)} ms</div></div>
              <div className="rounded border bg-muted p-2"><div className="text-muted-foreground">{t('debug.holdTime')}</div><div className="font-medium">{formatDuration(holdDurationSec)}</div></div>
              <div className="rounded border bg-muted p-2"><div className="text-muted-foreground">{t('debug.asrAudioTime')}</div><div className="font-medium">{formatDuration(asrDurationSec)}</div></div>
              <div className="rounded border bg-muted p-2"><div className="text-muted-foreground">LLM Provider</div><div className="font-medium">{prompts.provider}</div></div>
            </div>

            <div className="rounded border bg-muted p-3 text-xs">
              <p className="mb-2 font-medium text-foreground">{t('debug.sessionParams')}</p>
              <div className="grid gap-x-4 gap-y-1 md:grid-cols-2">
                <div className="text-muted-foreground">connection_id</div>
                <div className="break-all font-mono text-xs">{ready?.connection_id || '-'}</div>
                <div className="text-muted-foreground">{t('debug.backendCapabilities')}</div>
                <div>ASR: {ready?.asr ? 'ON' : 'OFF'} / LLM: {ready?.llm ? 'ON' : 'OFF'}</div>
                <div className="text-muted-foreground">{t('debug.audioChunks')}</div>
                <div>{session.audioChunks.length} chunks</div>
                <div className="text-muted-foreground">{t('debug.audioBytes')}</div>
                <div>{totalBytes} bytes</div>
              </div>
            </div>

            <div className="space-y-2 rounded border bg-card p-3">
              <div className="flex items-center justify-between">
                <p className="text-xs font-medium">{t('debug.fullSystemPrompt')}</p>
              </div>
              <pre className="max-h-56 overflow-auto whitespace-pre-wrap break-all rounded bg-muted p-2 text-xs text-foreground">
                {prompts.systemPrompt || t('debug.none')}
              </pre>

              <p className="text-xs font-medium">{t('debug.fullUserPrompt')}</p>
              <pre className="max-h-56 overflow-auto whitespace-pre-wrap break-all rounded bg-muted p-2 text-xs text-foreground">
                {prompts.userPrompt || t('debug.none')}
              </pre>

              {prompts.rawOutput && (
                <>
                  <p className="text-xs font-medium">LLM Raw Output</p>
                  <pre className="max-h-40 overflow-auto whitespace-pre-wrap break-all rounded bg-muted p-2 text-xs text-foreground">
                    {prompts.rawOutput}
                  </pre>
                </>
              )}
            </div>

            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <p className="text-xs font-medium">{t('debug.sessionLog')}</p>
                <div className="flex items-center gap-2">
                  {!showAllLogs && filteredOutCount > 0 && (
                    <span className="text-xs text-muted-foreground">{t('debug.hiddenLogs', { count: filteredOutCount })}</span>
                  )}
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="h-7 px-2 text-xs"
                    onClick={() => setShowAllLogs((value) => !value)}
                  >
                    {t(showAllLogs ? 'debug.keyLogsOnly' : 'debug.showAllLogs')}
                  </Button>
                </div>
              </div>

              <div className="space-y-1.5">
                {logs.map((message, index) => (
                  <LogItem key={`${message.time}-${index}`} msg={message} />
                ))}
              </div>
            </div>
          </>
        )}
      </CardContent>
    </Card>
  )
}
