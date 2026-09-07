import { useState } from 'react'
import { Plus, X, Search, RotateCcw, ChevronDown, ChevronUp, FolderPlus, Trash2, Download, Info } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Switch } from '@/components/ui/switch'
import { Tooltip } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'
import { exportHotwords } from '@/services/exports'
import { BUILTIN_SETS, MAX_HOTWORDS } from '@/services/hotwords/model'
import { useHotwordsManager } from '@/services/hotwords/useHotwordsManager'
import TextReplacementSection from '@/components/TextReplacementSection'
import TextFormatSection from '@/components/TextFormatSection'
import { useSortable, DragHandle } from '@/components/ui/sortable'
import { t } from '@/i18n'
import { RichText } from '@/i18n/RichText'
import { useT } from '@/i18n/useT'

type Tab = 'hotwords' | 'replacement'

/** 超过该数量的热词总数时，给出"过多可能反而降低准确率"的软提示 */
const HOTWORD_SOFT_LIMIT = 200
/** 单个分类内词条超过该数量时折叠，只显示前 N 个 + 展开按钮，避免一大片平铺 */
const CHIPS_COLLAPSE_LIMIT = 30

/** 词条标签列表：数量多时折叠（搜索中不折叠，避免藏起匹配项）。 */
function WordChips({
  words,
  onRemove,
  expandAll = false,
}: {
  words: string[]
  onRemove: (word: string) => void
  expandAll?: boolean
}) {
  const [expanded, setExpanded] = useState(false)
  const overflow = words.length > CHIPS_COLLAPSE_LIMIT
  const shown = expanded || expandAll || !overflow ? words : words.slice(0, CHIPS_COLLAPSE_LIMIT)

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap gap-1.5">
        {shown.map((word) => (
          <span
            key={word}
            className="inline-flex items-center gap-1 rounded-md border bg-secondary/50 px-2 py-0.5 text-xs"
          >
            {word}
            <button
              onClick={() => onRemove(word)}
              className="rounded-full p-0.5 transition-colors hover:bg-destructive/10 hover:text-destructive"
              aria-label={t('dict.deleteWord', { word })}
            >
              <X className="h-2.5 w-2.5 text-muted-foreground" />
            </button>
          </span>
        ))}
      </div>
      {overflow && !expandAll && (
        <button
          type="button"
          onClick={() => setExpanded(!expanded)}
          className="text-xs text-muted-foreground transition-colors hover:text-foreground"
        >
          {expanded ? t('dict.collapse') : t('dict.expandAll', { count: words.length })}
        </button>
      )}
    </div>
  )
}

export default function Dictionary() {
  useT()
  const [tab, setTab] = useState<Tab>('hotwords')
  const [exportMessage, setExportMessage] = useState('')
  const [warnDismissed, setWarnDismissed] = useState(false)
  const {
    hotwords,
    builtinSetWords,
    builtinSetActive,
    customThemes,
    customThemeActive,
    themeInputs,
    newThemeName,
    search,
    loading,
    showUnknown,
    filtered,
    filteredUnknown,
    visibleCustomThemes,
    getSetWordsInHotwords,
    getThemeWordsInHotwords,
    setNewThemeName,
    setSearch,
    setShowUnknown,
    setThemeInput,
    addTheme,
    addWordsToTheme,
    removeTheme,
    moveTheme,
    toggleCustomTheme,
    removeWord,
    toggleBuiltinSet,
    resetBuiltinSet,
  } = useHotwordsManager()

  // 搜索时列表是过滤后的，序号对不上完整列表，因此搜索状态下不启用拖拽
  const themeSortable = useSortable({ onMove: (from, to) => void moveTheme(from, to) })

  const handleExport = async () => {
    const result = await exportHotwords()
    setExportMessage(result.canceled ? t('history.exportCanceled') : t('history.savedTo', { path: result.filePath ?? '' }))
  }

  return (
    <div className="mx-auto max-w-4xl">
      {/* 标题栏 + Tab */}
      <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-4">
          <h1 className="text-2xl font-bold">{t('nav.hotwords')}</h1>
          <div className="flex gap-1 rounded-lg border border-border p-0.5">
            <button
              type="button"
              onClick={() => setTab('hotwords')}
              className={cn(
                'rounded-md px-3 py-1 text-xs transition-colors',
                tab === 'hotwords' ? 'bg-accent font-medium text-foreground' : 'text-muted-foreground hover:text-foreground',
              )}
            >{t('dict.tabHotwords')}</button>
            <button
              type="button"
              onClick={() => setTab('replacement')}
              className={cn(
                'rounded-md px-3 py-1 text-xs transition-colors',
                tab === 'replacement' ? 'bg-accent font-medium text-foreground' : 'text-muted-foreground hover:text-foreground',
              )}
            >{t('dict.tabTextProcess')}</button>
          </div>
        </div>

        {/* 热词 Tab 的搜索和导出 */}
        {tab === 'hotwords' && (
          <div className="flex items-center gap-2">
            <span className="text-sm text-muted-foreground">
              {hotwords.length} / {MAX_HOTWORDS}
            </span>
            <div className="relative">
              <Search className="absolute left-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
              <input
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder={t('dict.searchPlaceholder')}
                className="w-52 rounded-md border border-input-border bg-input-bg py-1.5 pl-8 pr-3 text-sm"
              />
            </div>
            <Tooltip content={t('history.export')}>
              <Button
                variant="ghost"
                size="icon"
                className="h-8 w-8 text-muted-foreground transition-colors hover:bg-accent/60 hover:text-foreground"
                onClick={() => void handleExport()}
                aria-label={t('history.export')}
              >
                <Download className="h-4 w-4" />
              </Button>
            </Tooltip>
          </div>
        )}
      </div>

      {exportMessage && tab === 'hotwords' && (
        <p className="mb-2 text-sm text-muted-foreground">{exportMessage}</p>
      )}

      {/* ===== 文本处理 Tab ===== */}
      {tab === 'replacement' && (
        <>
          <TextFormatSection />
          <TextReplacementSection />
        </>
      )}

      {/* ===== 热词 Tab ===== */}
      {tab === 'hotwords' && (
        <>
          <p className="mb-4 flex items-center gap-1.5 text-sm text-muted-foreground">
            <span>{t('dict.intro')}</span>
            <Tooltip
              variant="light"
              content={
                <div className="text-left">
                  <p className="mb-1.5 font-medium">{t('dict.supportTitle')}</p>
                  <table className="border-collapse text-xs [&_td]:py-0.5 [&_td]:pr-3 [&_td]:align-top [&_th]:pb-1 [&_th]:pr-3 [&_th]:text-left [&_th]:font-normal [&_th]:text-muted-foreground/70">
                    <thead>
                      <tr><th>{t('dict.tableModel')}</th><th>{t('dict.tableHotword')}</th><th>{t('dict.tableNote')}</th></tr>
                    </thead>
                    <tbody>
                      {/* 模型名是品牌 + 型号，两种语言下都用官方写法，只有"支持/说明"两列走翻译 */}
                      <tr><td>Doubao Seed-ASR 2.0</td><td>{t('dict.support.yes')}</td><td>{t('dict.note.doubao')}</td></tr>
                      <tr><td>{t('dict.model.qwenAudio30')}</td><td>{t('dict.support.yes')}</td><td>{t('dict.note.qwenAudio30')}</td></tr>
                      <tr><td>{t('dict.model.qwenRealtime')}</td><td>{t('dict.support.yes')}</td><td>{t('dict.note.qwenRealtime')}</td></tr>
                      <tr><td>{t('dict.model.qwenFlash')}</td><td>{t('dict.support.yes')}</td><td>{t('dict.note.qwenFlash')}</td></tr>
                      <tr><td>{t('dict.model.localQwen')}</td><td>{t('dict.support.notYet')}</td><td>{t('dict.note.localQwen')}</td></tr>
                      <tr><td>{t('dict.model.localOther')}</td><td>{t('dict.support.no')}</td><td>{t('dict.note.localOther')}</td></tr>
                      <tr><td>{t('dict.model.server')}</td><td>{t('dict.support.yes')}</td><td>{t('dict.note.server')}</td></tr>
                    </tbody>
                  </table>
                  <p className="mt-2 max-w-[380px] leading-relaxed">
                    <RichText text={t('dict.caveat')} />
                  </p>
                </div>
              }
            >
              <Info className="h-3.5 w-3.5 shrink-0 cursor-help text-muted-foreground/50 transition-colors hover:text-muted-foreground" />
            </Tooltip>
          </p>

          {hotwords.length > HOTWORD_SOFT_LIMIT && !warnDismissed && (
            <div className="mb-4 -mt-2 flex items-start gap-2 text-xs text-amber-500">
              <p>
                <RichText text={t('dict.countHint', { count: hotwords.length })} />
              </p>
              <button
                type="button"
                onClick={() => setWarnDismissed(true)}
                className="shrink-0 rounded p-0.5 text-amber-500/70 transition-colors hover:bg-amber-500/10 hover:text-amber-500"
                aria-label={t('dict.dismissHint')}
              >
                <X className="h-3 w-3" />
              </button>
            </div>
          )}

          {/* 新建热词分类 */}
          <div className="mb-4 flex items-center gap-2">
            <FolderPlus className="h-4 w-4 shrink-0 text-muted-foreground" />
            <input
              value={newThemeName}
              onChange={(e) => setNewThemeName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault()
                  void addTheme()
                }
              }}
              placeholder={t('dict.newCategoryPlaceholder')}
              className="flex-1 rounded-md border border-input-border bg-input-bg px-3 py-1.5 text-sm"
            />
            <Button
              onClick={() => void addTheme()}
              size="sm"
              variant="outline"
              disabled={!newThemeName.trim()}
              className="shrink-0 gap-1.5"
            >
              <Plus className="h-3.5 w-3.5" />
              {t('dict.add')}
            </Button>
          </div>

          {loading ? (
            <p className="py-8 text-center text-muted-foreground">{t('dict.loading')}</p>
          ) : (
            <div className="space-y-4">
              {/* 自定义分类 */}
              {customThemes.length > 1 && !search && (
                <p className="text-xs text-muted-foreground/70">
                  {t('dict.orderHint')}
                </p>
              )}
              {visibleCustomThemes.map((theme, themeIndex) => {
                const canSort = !search && customThemes.length > 1
                const active = !!customThemeActive[theme.id]
                const activeWords = getThemeWordsInHotwords(theme)
                const totalWords = theme.words.length

                return (
                  <Card
                    key={theme.id}
                    {...(canSort ? themeSortable.rowProps(themeIndex) : {})}
                    className={cn('group', !active && 'border-dashed opacity-70', canSort && themeSortable.rowClassName(themeIndex))}
                  >
                    <CardContent className="p-4">
                      <div className="mb-3 flex items-center justify-between gap-3">
                        <div className="flex items-center gap-3 min-w-0">
                          {canSort && <DragHandle {...themeSortable.handleProps(themeIndex, t('dict.dragCategory', { name: theme.name }))} />}
                          <Switch
                            checked={active}
                            onChange={() => void toggleCustomTheme(theme.id)}
                            size="sm"
                          />
                          <div className="min-w-0">
                            <p className="text-sm font-medium">{theme.name}</p>
                            <p className="text-xs text-muted-foreground">
                              {t('dict.customCount', { active: activeWords.length, total: totalWords })}
                            </p>
                          </div>
                        </div>
                        <div className="flex shrink-0 items-center">
                          <Tooltip content={t('dict.deleteCategory')}>
                            <button
                              type="button"
                              onClick={() => void removeTheme(theme.id)}
                              className="rounded p-1.5 text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
                              aria-label={t('dict.deleteTheme', { name: theme.name })}
                            >
                              <Trash2 className="h-3.5 w-3.5" />
                            </button>
                          </Tooltip>
                        </div>
                      </div>

                      {active && (
                        <div className="space-y-3">
                          <div className="flex gap-2">
                            <input
                              value={themeInputs[theme.id] || ''}
                              onChange={(e) => setThemeInput(theme.id, e.target.value)}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter' && !e.shiftKey) {
                                  e.preventDefault()
                                  void addWordsToTheme(theme.id)
                                }
                              }}
                              placeholder={t('dict.addWordsPlaceholder')}
                              className="flex-1 rounded-md border border-input-border bg-input-bg px-3 py-1.5 text-sm"
                            />
                            <Button
                              onClick={() => void addWordsToTheme(theme.id)}
                              size="sm"
                              variant="outline"
                              disabled={!(themeInputs[theme.id] || '').trim()}
                              className="shrink-0 gap-1.5"
                            >
                              <Plus className="h-3.5 w-3.5" />
                              {t('dict.add')}
                            </Button>
                          </div>

                          {activeWords.length > 0 && (
                            <WordChips words={activeWords} onRemove={removeWord} expandAll={!!search} />
                          )}
                        </div>
                      )}
                    </CardContent>
                  </Card>
                )
              })}

              {/* 内置分类 */}
              {Object.entries(BUILTIN_SETS).map(([key, setDef]) => {
                const active = !!builtinSetActive[key]
                const activeWords = getSetWordsInHotwords(key)
                const totalWords = (builtinSetWords[key] || []).length

                if (search && activeWords.length === 0 && !setDef.label.toLowerCase().includes(search.toLowerCase())) {
                  return null
                }

                return (
                  <Card key={key} className={cn(!active && 'border-dashed opacity-70')}>
                    <CardContent className="p-4">
                      <div className="mb-3 flex items-center justify-between gap-3">
                        <div className="flex items-center gap-3 min-w-0">
                          <Switch
                            checked={active}
                            onChange={() => void toggleBuiltinSet(key)}
                            size="sm"
                          />
                          <div className="min-w-0">
                            <p className="text-sm font-medium">{setDef.label}</p>
                            <p className="text-xs text-muted-foreground">
                              {t('dict.builtinCount', { desc: setDef.description, active: activeWords.length, total: totalWords })}
                            </p>
                          </div>
                        </div>
                        <Button
                          size="sm"
                          variant="outline"
                          className="h-7 gap-1 px-2 text-xs"
                          onClick={() => void resetBuiltinSet(key)}
                        >
                          <RotateCcw className="h-3 w-3" /> {t('dict.reset')}
                        </Button>
                      </div>

                      {active && activeWords.length > 0 && (
                        <WordChips words={activeWords} onRemove={removeWord} expandAll={!!search} />
                      )}
                    </CardContent>
                  </Card>
                )
              })}

              {/* 历史未分类词汇 */}
              {filteredUnknown.length > 0 && (
                <Card>
                  <CardContent className="p-4">
                    <button
                      className="mb-2 flex w-full items-center justify-between text-left"
                      onClick={() => setShowUnknown(!showUnknown)}
                    >
                      <div>
                        <p className="text-sm font-medium">{t('dict.legacyTitle')}</p>
                        <p className="text-xs text-muted-foreground">
                          {t('dict.legacyDesc')}
                        </p>
                      </div>
                      <div className="flex items-center gap-2">
                        <span className="text-xs text-muted-foreground">{filteredUnknown.length}</span>
                        {showUnknown ? <ChevronUp className="h-4 w-4 text-muted-foreground" /> : <ChevronDown className="h-4 w-4 text-muted-foreground" />}
                      </div>
                    </button>

                    {showUnknown && (
                      <WordChips words={filteredUnknown} onRemove={removeWord} expandAll={!!search} />
                    )}
                  </CardContent>
                </Card>
              )}

              {!search && hotwords.length === 0 && (
                <div className="rounded-lg border border-dashed border-border py-8 text-center">
                  <p className="text-sm text-muted-foreground">
                    {t('dict.empty')}
                  </p>
                </div>
              )}

              {search && filtered.length === 0 && (
                <p className="py-8 text-center text-muted-foreground">{t('dict.noMatch')}</p>
              )}
            </div>
          )}
        </>
      )}
    </div>
  )
}
