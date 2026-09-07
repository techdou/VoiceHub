// 文本替换规则管理组件

import { useCallback, useEffect, useRef, useState } from 'react'
import { Plus, X, List } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Switch } from '@/components/ui/switch'
import { useSortable, DragHandle, moveItem } from '@/components/ui/sortable'
import { t } from '@/i18n'
import { useT } from '@/i18n/useT'
import {
  getTextReplacements,
  saveTextReplacements,
  parseBatchReplacements,
  type TextReplacementRule,
} from '@/services/textReplacement'

function createId() {
  return Date.now().toString(36) + Math.random().toString(36).slice(2, 6)
}

export default function TextReplacementSection() {
  useT()
  const [rules, setRules] = useState<TextReplacementRule[]>([])
  const [fromInput, setFromInput] = useState('')
  const [toInput, setToInput] = useState('')
  const [batchMode, setBatchMode] = useState(false)
  const [batchText, setBatchText] = useState('')
  const [batchHint, setBatchHint] = useState('')

  useEffect(() => {
    getTextReplacements().then((loaded) => {
      // 兼容旧数据：确保每条规则都有唯一 id 和 enabled 字段
      const migrated = loaded.map((r) => ({
        ...r,
        id: r.id || createId(),
        enabled: r.enabled !== false,
      }))
      setRules(migrated)
    })
  }, [])

  const addRule = useCallback(() => {
    const from = fromInput.trim()
    const to = toInput.trim()
    if (!from) return
    setRules((prev) => {
      if (prev.some((r) => r.from === from)) return prev
      return [...prev, { id: createId(), from, to, enabled: true }]
    })
    setFromInput('')
    setToInput('')
  }, [fromInput, toInput])

  const importBatch = useCallback(() => {
    const parsed = parseBatchReplacements(batchText)
    if (parsed.length === 0) {
      setBatchHint(t('textReplace.importNothing'))
      return
    }
    setRules((prev) => {
      const existing = new Set(prev.map((r) => r.from))
      const next = [...prev]
      let added = 0
      let updated = 0
      for (const { from, to } of parsed) {
        if (existing.has(from)) {
          // 已存在同一原文：更新其替换内容
          const idx = next.findIndex((r) => r.from === from)
          if (idx >= 0 && next[idx].to !== to) {
            next[idx] = { ...next[idx], to }
            updated++
          }
        } else {
          next.push({ id: createId(), from, to, enabled: true })
          existing.add(from)
          added++
        }
      }
      setBatchHint(updated > 0
        ? t('textReplace.importedWithUpdates', { added, updated })
        : t('textReplace.imported', { added }))
      return next
    })
    setBatchText('')
  }, [batchText])

  /** 拖拽排序。顺序即执行顺序（applyReplacements 按数组顺序逐条替换）。 */
  const moveRule = useCallback((from: number, to: number) => {
    setRules((prev) => moveItem(prev, from, to))
  }, [])

  const sortable = useSortable({ onMove: moveRule })

  const removeRule = useCallback((id: string) => {
    setRules((prev) => prev.filter((r) => r.id !== id))
  }, [])

  const toggleRule = useCallback((id: string) => {
    setRules((prev) => {
      const next = prev.map((r) => r.id === id ? { ...r, enabled: !r.enabled } : r)
      return next
    })
  }, [])

  // 每次 rules 变化后持久化
  const initialized = useRef(false)
  useEffect(() => {
    if (!initialized.current) {
      initialized.current = true
      return
    }
    void saveTextReplacements(rules)
  }, [rules])

  const enabledCount = rules.filter((r) => r.enabled).length

  return (
    <div className="rounded-lg border border-border">
      <div className="border-b border-border px-4 py-3">
        <h2 className="text-lg font-semibold">{t('textReplace.title')}</h2>
        <p className="mt-0.5 text-xs text-muted-foreground">
          {t('textReplace.desc')}
          {rules.length > 0 && t('textReplace.counts', { enabled: enabledCount, total: rules.length })}
        </p>
        {rules.length > 1 && (
          <p className="mt-1 text-xs text-muted-foreground/70">
            {t('textReplace.orderHint')}
          </p>
        )}
      </div>
      <div className="p-4">
        {/* 添加新规则 — 两列对齐 */}
        {!batchMode && (
          <div className="flex items-center gap-2">
            <input
              value={fromInput}
              onChange={(e) => setFromInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault()
                  void addRule()
                }
              }}
              placeholder={t('textReplace.from')}
              className="w-0 flex-1 rounded-md border border-input-border bg-input-bg px-3 py-1.5 text-sm"
            />
            <input
              value={toInput}
              onChange={(e) => setToInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault()
                  void addRule()
                }
              }}
              placeholder={t('textReplace.to')}
              className="w-0 flex-1 rounded-md border border-input-border bg-input-bg px-3 py-1.5 text-sm"
            />
            <Button
              onClick={() => void addRule()}
              size="sm"
              variant="outline"
              disabled={!fromInput.trim()}
              className="shrink-0 gap-1.5"
            >
              <Plus className="h-3.5 w-3.5" />
              {t('textReplace.add')}
            </Button>
            <Button
              onClick={() => { setBatchMode(true); setBatchHint('') }}
              size="sm"
              variant="outline"
              className="shrink-0 gap-1.5"
            >
              <List className="h-3.5 w-3.5" />
              {t('textReplace.batch')}
            </Button>
          </div>
        )}

        {/* 批量添加 */}
        {batchMode && (
          <div className="rounded-lg border border-border p-3">
            <p className="mb-2 text-xs text-muted-foreground">
              {/* 按 {arrow} 占位切分，让 <code>=&gt;</code> 落在译文决定的位置上 */}
              {t('textReplace.batchHelp').split('{arrow}').map((part, i) => (
                <span key={i}>{i > 0 && <code>=&gt;</code>}{part}</span>
              ))}
              <br />
              {t('textReplace.batchExample')}
              <br />
              Cloud Code =&gt; Claude Code
              <br />
              {t('textReplace.batchDeleteHint')}
            </p>
            <textarea
              value={batchText}
              onChange={(e) => setBatchText(e.target.value)}
              rows={6}
              placeholder={t('textReplace.batchPlaceholder')}
              className="w-full resize-y rounded-md border border-input-border bg-input-bg px-3 py-2 text-sm leading-relaxed"
            />
            <div className="mt-2 flex items-center justify-between gap-2">
              <span className="text-xs text-muted-foreground">{batchHint}</span>
              <div className="flex shrink-0 gap-2">
                <Button
                  onClick={() => { setBatchMode(false); setBatchText(''); setBatchHint('') }}
                  size="sm"
                  variant="outline"
                >
                  {t('textReplace.done')}
                </Button>
                <Button
                  onClick={() => void importBatch()}
                  size="sm"
                  variant="outline"
                  disabled={!batchText.trim()}
                  className="gap-1.5"
                >
                  <Plus className="h-3.5 w-3.5" />
                  {t('textReplace.import')}
                </Button>
              </div>
            </div>
          </div>
        )}

        {/* 规则列表 — 两列布局 */}
        {rules.length > 0 && (
          <div className="mt-4 rounded-lg border border-border overflow-hidden">
            {/* 列头 */}
            <div className="flex items-center border-b border-border bg-muted/30 px-3 py-1.5">
              <span className="w-[22px] shrink-0" />
              <span className="w-7 shrink-0" />
              <span className="flex-1 pl-3 text-xs text-muted-foreground">{t('textReplace.from')}</span>
              <span className="mx-3 h-3 w-px shrink-0 bg-border" />
              <span className="flex-1 text-xs text-muted-foreground">{t('textReplace.toShort')}</span>
              <span className="w-8 shrink-0" />
            </div>
            {rules.map((rule, index) => (
              <div
                key={rule.id}
                {...sortable.rowProps(index)}
                className={`group flex items-center border-t border-border/60 px-3 py-2 transition-colors hover:bg-accent/40 first:border-t-0 ${!rule.enabled ? 'opacity-50' : ''
                  } ${sortable.rowClassName(index)}`}
              >
                <DragHandle {...sortable.handleProps(index, t('textReplace.dragHandle', { name: rule.from }))} />
                <Switch
                  checked={rule.enabled}
                  onChange={() => void toggleRule(rule.id)}
                  size="sm"
                  className="shrink-0"
                />
                <span className={`flex-1 truncate pl-3 text-sm ${!rule.enabled ? 'line-through text-muted-foreground' : ''}`}>
                  {rule.from}
                </span>
                <span className="mx-3 h-4 w-px shrink-0 bg-border/50" />
                <span className="flex-1 truncate text-sm text-muted-foreground">
                  {rule.to || <span className="italic text-muted-foreground/50">{t('textReplace.deleteMark')}</span>}
                </span>
                <button
                  onClick={() => void removeRule(rule.id)}
                  className="w-8 shrink-0 flex justify-center rounded-full p-1 text-muted-foreground/0 transition-colors group-hover:text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                  aria-label={t('textReplace.deleteRule', { name: rule.from })}
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              </div>
            ))}
          </div>
        )}

        {rules.length === 0 && (
          <div className="mt-6 rounded-lg border border-dashed border-border py-8 text-center">
            <p className="text-sm text-muted-foreground">{t('textReplace.empty')}</p>
          </div>
        )}
      </div>
    </div>
  )
}
