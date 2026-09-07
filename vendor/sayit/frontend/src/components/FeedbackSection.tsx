// 首页意见反馈卡片

import { useEffect, useState } from 'react'
import { X } from 'lucide-react'
import { open } from '@tauri-apps/plugin-shell'
import { getBackendBaseUrl } from '@/services/runtimeConfig'
import { getLastTranscript, submitFeedback } from '@/services/feedback'
import { t } from '@/i18n'
import { useT } from '@/i18n/useT'

export default function FeedbackSection() {
  useT()
  const [lastTranscript, setLastTranscript] = useState<string>('')
  const [showTranscript, setShowTranscript] = useState(true)
  const [feedbackText, setFeedbackText] = useState('')
  const [sending, setSending] = useState(false)
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null)
  const [submitted, setSubmitted] = useState(false)

  // 公开反馈页地址 = 用户在设置里填写的服务器地址 + /feedback.html
  const feedbackUrl = `${getBackendBaseUrl()}/feedback.html`

  useEffect(() => {
    getLastTranscript().then((record) => {
      if (record) {
        if (record.isEmpty) {
          setLastTranscript(t('record.noSpeech'))
        } else {
          const display = record.llmText || record.asrText || ''
          setLastTranscript(display.slice(0, 200))
        }
      }
    })
  }, [])

  const handleSubmit = async () => {
    if (sending) return
    const trimmed = feedbackText.trim()
    if (trimmed.length < 2) {
      setMessage({ ok: false, text: t('feedback.tooShort') })
      return
    }

    setSending(true)
    setMessage(null)
    try {
      const result = await submitFeedback(trimmed, { includeTranscript: showTranscript && !!lastTranscript })
      if (result.ok) {
        setFeedbackText('')
        setSubmitted(true) // 显示常驻的“发送成功 + 查看进度”行
      } else {
        setSubmitted(false)
        setMessage({ ok: false, text: result.message })
      }
    } catch (err) {
      setSubmitted(false)
      setMessage({ ok: false, text: t('feedback.networkError') })
    } finally {
      setSending(false)
    }
  }

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      handleSubmit()
    }
  }

  const handleOpenFeedbackPage = async () => {
    try {
      await open(feedbackUrl)
    } catch {
      // 打开外部浏览器失败时静默忽略，不影响反馈提交流程
    }
  }

  return (
    <div>
      <h2 className="mb-3 text-lg font-semibold">{t('feedback.title')}</h2>

      <div className="rounded-xl border border-border p-4">
        {/* 转录引用块 — 可删除 */}
        {showTranscript && lastTranscript && (
          <div className="relative mb-3">
            <div className="rounded-lg bg-muted/70 px-3 py-2 pr-8">
              <p className="text-sm text-muted-foreground">{t('feedback.lastTranscript')}</p>
              <p className="mt-1 truncate border-l-2 border-muted-foreground/30 pl-2 text-sm text-muted-foreground/70">{lastTranscript}</p>
            </div>
            <button
              onClick={() => setShowTranscript(false)}
              className="absolute right-2 top-2 rounded p-0.5 text-muted-foreground/50 transition-colors hover:text-muted-foreground"
              aria-label={t('feedback.removeTranscript')}
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        )}

        {/* 反馈输入 */}
        <textarea
          value={feedbackText}
          onChange={(e) => {
            setFeedbackText(e.target.value)
            if (submitted) setSubmitted(false)
            if (message) setMessage(null)
          }}
          onKeyDown={handleKeyDown}
          placeholder={t('feedback.placeholder')}
          rows={1}
          maxLength={1000}
          className="w-full resize-none border-0 bg-transparent p-0 text-sm outline-none placeholder:text-muted-foreground/50"
          style={{ fieldSizing: 'content' as never, minHeight: '1.5rem', maxHeight: '6rem' }}
        />

        {/* 底部：状态提示（错误 / 成功）在左，发送按钮在右——成功提示也在此处显示，不再额外撑高卡片 */}
        <div className="mt-3 flex items-center justify-between gap-3">
          <div className="min-h-[1.25rem] flex-1 text-xs leading-relaxed">
            {submitted ? (
              <p className="text-muted-foreground">
                <span className="text-green-600 dark:text-green-400">{t('feedback.sent')}</span>
                {t('feedback.sentAfter')}{' '}
                <button
                  onClick={handleOpenFeedbackPage}
                  className="text-foreground transition-opacity hover:opacity-70"
                >
                  <span className="break-all underline underline-offset-2">{feedbackUrl}</span>
                  <sup className="ml-0.5">↗</sup>
                </button>
                {t('feedback.sentLinkSuffix')}
              </p>
            ) : (
              message && (
                <p className={message.ok ? 'text-green-600 dark:text-green-400' : 'text-destructive'}>
                  {message.text}
                </p>
              )
            )}
          </div>
          <button
            onClick={handleSubmit}
            disabled={sending || feedbackText.trim().length < 2}
            className="shrink-0 rounded-full bg-secondary px-4 py-1.5 text-xs font-medium text-foreground transition-colors hover:bg-foreground hover:text-background disabled:opacity-40"
          >
            {sending ? t('feedback.sending') : t('feedback.send')}
          </button>
        </div>
      </div>
    </div>
  )
}
