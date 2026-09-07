import { useCallback, useEffect, useRef, useState } from 'react'
import { AlertTriangle, Check, Info, Loader2, Pause, Play, RotateCcw } from 'lucide-react'
import { Modal } from '@/components/ui/modal'
import { Button } from '@/components/ui/button'
import * as bridge from '@/services/bridge'
import { type HistoryRecord } from '@/services/store'
import {
  ASR_CORRECTION_WITHDRAW_DAYS,
  grantAsrCorrectionConsent,
  hasAsrCorrectionConsent,
  submitAsrCorrection,
  withdrawAsrCorrection,
} from '@/services/asrCorrection'
import { normalizeForDiff } from '@/lib/asrDiff'
import { t } from '@/i18n'
import { useT } from '@/i18n/useT'
import { useRecordingPlayback } from './useRecordingPlayback'
import { AudioProgressBar } from './AudioProgressBar'
import { AsrDiffPreview } from './AsrDiffPreview'

/**
 * 「纠正识别」面板。
 *
 * 三个刻意的设计，改之前先看 .kiro/decisions.md：
 * 1. 可编辑的那一栏初值是 **record.asrText（识别原文）**，绝不是 llmText。
 *    用户平时看到的是 AI 整理后的文本，如果照着它改（补标点、分段、改书面语），
 *    拿到的标注对 ASR 训练是噪声，因此纠正面板不展示 AI 整理后的文本。
 * 2. 音频读不到就不让提交。没有音频的样本对识别改进没有用。
 * 3. 首次提交必须过一次明确同意（不是一个信息图标）。上传的是用户的录音，
 *    比文本敏感得多。
 */
/**
 * 可编辑框的高度上限（px）。上面那块「识别原文」是 max-h-40（160px），这里给到 200px：
 * 两块的行高、字号、内边距完全一致，所以同样的文字在两块里占一样的高度，
 * 只是"要动手改"的那块允许多长一点。超过之后内部滚动，而不是把弹窗撑到屏幕外。
 */
const EDITOR_MAX_HEIGHT_PX = 200

export function AsrCorrectionDialog({
  record,
  onClose,
  onSubmitted,
}: {
  record: HistoryRecord
  onClose: () => void
  onSubmitted: (patch: Partial<HistoryRecord>) => void
}) {
  useT()
  const originalText = record.asrText || ''
  const [correctedText, setCorrectedText] = useState(record.asrCorrectedText || originalText)
  const [audioState, setAudioState] = useState<'checking' | 'ok' | 'missing'>('checking')
  const [consentNeeded, setConsentNeeded] = useState(false)
  const [consentChecked, setConsentChecked] = useState(false)
  const [submitting, setSubmitting] = useState(false)
  const [withdrawing, setWithdrawing] = useState(false)
  const [error, setError] = useState('')
  const editorRef = useRef<HTMLTextAreaElement | null>(null)
  // 用记录里存的音频时长做初值，这样没点播放也能看到总长（audioDurationSec 是音频真实
  // 长度，durationSec 是按住快捷键的时长，只在老记录缺前者时兜底）
  const playback = useRecordingPlayback(
    record.audioFilePath,
    record.audioDurationSec || record.durationSec || 0,
  )

  const submitted = Boolean(record.asrCorrectionId)

  useEffect(() => {
    let alive = true
    void (async () => {
      const path = record.audioFilePath
      if (!path) {
        if (alive) setAudioState('missing')
        return
      }
      const exists = await bridge.audioFileExists(path).catch(() => false)
      if (alive) setAudioState(exists ? 'ok' : 'missing')
    })()
    void (async () => {
      const granted = await hasAsrCorrectionConsent().catch(() => false)
      if (alive) setConsentNeeded(!granted)
    })()
    return () => { alive = false }
  }, [record.audioFilePath])

  /**
   * 让可编辑框跟着内容长高，行为对齐上面那块只读的「识别原文」：一行内容就是一行高，
   * 到上限后内部滚动。
   *
   * 不用 `rows={行数}` 那种算法：`rows` 只数换行符，一段没有换行但会折行的长文本
   * 仍然只算一行，框会比内容矮一大截。所以按 scrollHeight 实测。
   *
   * `+ extra` 是上下边框：box-sizing 是 border-box，height 含边框而 scrollHeight 不含，
   * 少了它每次都会矮 2px，于是内容一进来就立刻出现滚动条。
   */
  useEffect(() => {
    const el = editorRef.current
    if (!el) return
    el.style.height = 'auto'
    const extra = el.offsetHeight - el.clientHeight
    el.style.height = `${Math.min(el.scrollHeight + extra, EDITOR_MAX_HEIGHT_PX)}px`
  }, [correctedText])

  const changed = normalizeForDiff(originalText) !== normalizeForDiff(correctedText)
  const canSubmit =
    !submitted &&
    audioState === 'ok' &&
    changed &&
    !submitting &&
    (!consentNeeded || consentChecked)

  const handleSubmit = useCallback(async () => {
    setError('')
    setSubmitting(true)
    try {
      if (consentNeeded) await grantAsrCorrectionConsent()
      const result = await submitAsrCorrection(record, correctedText)
      if (!result.ok) {
        setError(result.message)
        return
      }
      setConsentNeeded(false)
      onSubmitted({
        asrCorrectionId: result.correctionId,
        asrCorrectionSubmittedAt: Date.now(),
        asrCorrectedText: normalizeForDiff(correctedText),
      })
    } finally {
      setSubmitting(false)
    }
  }, [consentNeeded, correctedText, onSubmitted, record])

  const handleWithdraw = useCallback(async () => {
    if (!record.asrCorrectionId) return
    setError('')
    setWithdrawing(true)
    try {
      const ok = await withdrawAsrCorrection(record.asrCorrectionId)
      if (!ok) {
        setError(t('asrCorrection.errorNetwork'))
        return
      }
      // 本地也清掉，入口回到"可提交"。
      // 必须写 null 而不是 undefined：patch 要过 JSON.stringify，undefined 的键会被丢掉，
      // 结果是字段没被清、界面一直显示"已提交"（见 HistoryRecord 上的注释）。
      onSubmitted({
        asrCorrectionId: null,
        asrCorrectionSubmittedAt: null,
        asrCorrectedText: null,
      })
    } finally {
      setWithdrawing(false)
    }
  }, [onSubmitted, record.asrCorrectionId])

  return (
    <Modal title={t('asrCorrection.title')} onClose={onClose} showCloseButton panelClassName="w-[620px]">
      <div className="mt-3 space-y-3">
        {/* 录音 */}
        {audioState === 'missing' ? (
          <div className="flex items-start gap-2 rounded-md border border-border bg-muted/40 px-2.5 py-2 text-xs text-muted-foreground">
            <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-500" aria-hidden />
            <span>{t('asrCorrection.audioMissing')}</span>
          </div>
        ) : (
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => { void playback.toggle() }}
              disabled={playback.loading || audioState !== 'ok'}
              className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-border hover:bg-accent disabled:opacity-50"
              aria-label={playback.playing ? t('record.pause') : t('record.play')}
            >
              {playback.loading ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" aria-hidden />
              ) : playback.playing ? (
                <Pause className="h-3.5 w-3.5 text-primary" aria-hidden />
              ) : (
                <Play className="h-3.5 w-3.5 text-muted-foreground" aria-hidden />
              )}
            </button>
            <AudioProgressBar playback={playback} className="min-w-0 flex-1" />
          </div>
        )}

        {/* 识别原文 + 改动标记合成一块。
            原本是"只读原文"和"改动预览"两个框，但 diff 本身就包含完整原文（删掉的部分
            带删除线），两个框摆在一起是同一段话看两遍 —— 连上可编辑框一共三个框，
            光是找"我该在哪儿打字"就要费一下。合成一块之后只剩「看」和「改」两块。 */}
        <AsrDiffPreview original={originalText} corrected={correctedText} />

        {/* 正确文本（可编辑） */}
        <div>
          <div className="mb-1 flex items-center justify-between gap-2">
            <label htmlFor="asr-correction-input" className="block text-xs font-medium text-foreground">
              {t('asrCorrection.correctedLabel')}
            </label>
            {/* 改乱了想重来时不用手抄一遍原文 */}
            <button
              type="button"
              onClick={() => setCorrectedText(originalText)}
              disabled={!changed || submitted || submitting}
              className="flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
            >
              <RotateCcw className="h-3 w-3" aria-hidden />
              {t('asrCorrection.reset')}
            </button>
          </div>
          {/* 内边距/字号/行高与「识别原文」那块保持一致，两块才对得齐；
              高度交给上面那个 effect 算，所以关掉手动拖拽（拖完一打字就会被算回去）。 */}
          <textarea
            ref={editorRef}
            id="asr-correction-input"
            data-modal-autofocus
            value={correctedText}
            onChange={(e) => setCorrectedText(e.target.value)}
            disabled={submitted || submitting}
            rows={1}
            className="w-full resize-none overflow-y-auto rounded-md border border-input-border bg-input-bg px-2.5 py-2 text-sm leading-relaxed focus:border-input-focus-border focus:outline-none disabled:opacity-60"
          />
          <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">{t('asrCorrection.correctedHint')}</p>
        </div>

        {/* 同意：首次提交必须勾；之后只留一个信息图标 */}
        {!submitted && (consentNeeded ? (
          <label className="flex cursor-pointer items-start gap-2 rounded-md border border-border bg-muted/30 px-2.5 py-2 text-xs leading-relaxed">
            <input
              type="checkbox"
              checked={consentChecked}
              onChange={(e) => setConsentChecked(e.target.checked)}
              className="mt-0.5 h-3.5 w-3.5 shrink-0 accent-primary"
            />
            <span className="text-muted-foreground">
              {t('asrCorrection.consent', { days: ASR_CORRECTION_WITHDRAW_DAYS })}
            </span>
          </label>
        ) : (
          <div className="flex items-start gap-1.5 text-[11px] leading-relaxed text-muted-foreground">
            <Info className="mt-0.5 h-3 w-3 shrink-0" aria-hidden />
            <span>{t('asrCorrection.consent', { days: ASR_CORRECTION_WITHDRAW_DAYS })}</span>
          </div>
        ))}

        {error && <p className="text-xs text-destructive">{error}</p>}

        <div className="flex items-center justify-end gap-2 pt-1">
          {submitted ? (
            <>
              <span className="mr-auto flex items-center gap-1.5 text-xs text-success">
                <Check className="h-3.5 w-3.5" aria-hidden />
                {t('asrCorrection.submittedWithId', { id: record.asrCorrectionId || '' })}
              </span>
              <Button variant="outline" size="sm" onClick={() => { void handleWithdraw() }} disabled={withdrawing}>
                {withdrawing && <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" aria-hidden />}
                {t('asrCorrection.withdraw')}
              </Button>
              <Button size="sm" onClick={onClose}>{t('common.close')}</Button>
            </>
          ) : (
            <>
              <Button variant="ghost" size="sm" onClick={onClose} disabled={submitting}>
                {t('common.cancel')}
              </Button>
              <Button size="sm" onClick={() => { void handleSubmit() }} disabled={!canSubmit}>
                {submitting && <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" aria-hidden />}
                {t('asrCorrection.submit')}
              </Button>
            </>
          )}
        </div>
      </div>
    </Modal>
  )
}
