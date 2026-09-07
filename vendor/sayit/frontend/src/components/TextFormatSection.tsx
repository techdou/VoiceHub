// 文本格式规范开关 — 不依赖 AI 的客户端文本处理

import { useEffect, useRef, useState } from 'react'
import { HelpCircle } from 'lucide-react'
import { Switch } from '@/components/ui/switch'
import { Tooltip } from '@/components/ui/tooltip'
import {
  getTextPostProcessOptions,
  saveTextPostProcessOptions,
  DEFAULT_POST_PROCESS,
  type TextPostProcessOptions,
} from '@/services/textPostProcess'
import type { TranslationKey } from '@/i18n'
import { useT } from '@/i18n/useT'

interface ToggleDef {
  key: keyof TextPostProcessOptions
  titleKey: TranslationKey
  /** 行内可见的简短说明 */
  hintKey: TranslationKey
  /** hover 问号显示的详细说明 + 例子（换行用 \n） */
  detailKey: TranslationKey
}

/**
 * 存 key 而不是文案。
 *
 * 这里的 detail 是「说明 + 例子 + AI 生效条件」三段拼起来的，所以拼接也放到渲染期，
 * 否则模块加载时就把当时的语言固化下来了。
 */
const TOGGLES: ToggleDef[] = [
  {
    key: 'autoSegment',
    titleKey: 'textFormat.autoSegment.title',
    hintKey: 'textFormat.autoSegment.hint',
    detailKey: 'textFormat.autoSegment.help',
  },
  {
    key: 'normalizeNumbers',
    titleKey: 'textFormat.normalizeNumbers.title',
    hintKey: 'textFormat.normalizeNumbers.hint',
    detailKey: 'textFormat.normalizeNumbers.help',
  },
  {
    key: 'stripTrailingPunctuation',
    titleKey: 'textFormat.stripTrailing.title',
    hintKey: 'textFormat.stripTrailing.hint',
    detailKey: 'textFormat.stripTrailing.help',
  },
  {
    key: 'punctuationToSpace',
    titleKey: 'textFormat.punctuationToSpace.title',
    hintKey: 'textFormat.punctuationToSpace.hint',
    detailKey: 'textFormat.punctuationToSpace.help',
  },
]

export default function TextFormatSection() {
  const t = useT()
  const [opts, setOpts] = useState<TextPostProcessOptions>(DEFAULT_POST_PROCESS)
  const initialized = useRef(false)
  // 读到已保存值之前，开关先隐藏、且不放动画：避免先画出默认值再跳到已保存值（闪一下）。
  // 用 finally 兜底，读取失败也让开关出现（呈默认值），不至于一直隐藏。
  const [ready, setReady] = useState(false)
  // animate 与 ready 分开：ready 决定何时显示，animate 决定何时允许过渡。
  // 若在揭开/赋值的同一帧就把 transition 加回来，按 CSS 规范浏览器会认为
  // 「有过渡且值变了」，于是把「默认值→已保存值」真的动画一遍（看起来就是闪一下）。
  // 所以揭开那一帧仍不带过渡，隔两帧待值稳定后才开过渡。
  const [animate, setAnimate] = useState(false)

  useEffect(() => {
    getTextPostProcessOptions()
      .then((loaded) => {
        setOpts(loaded)
        initialized.current = true
      })
      .finally(() => {
        setReady(true)
        requestAnimationFrame(() => requestAnimationFrame(() => setAnimate(true)))
      })
  }, [])

  useEffect(() => {
    if (!initialized.current) return
    void saveTextPostProcessOptions(opts)
  }, [opts])

  const toggle = (key: keyof TextPostProcessOptions) => {
    setOpts((prev) => ({ ...prev, [key]: !prev[key] }))
  }

  return (
    <div className="mb-6 rounded-lg border border-border">
      <div className="border-b border-border px-4 py-3">
        <h2 className="text-lg font-semibold">{t('textFormat.title')}</h2>
        {/* 说清生效条件：这几项是"没有 AI 时我们自己做的排版兜底"。开了 AI 整理，
            排版就归 AI 管，这里整体不生效——两边都做会互相打架。文本替换不在此列，见热词页的「文本替换」。 */}
        <p className="mt-0.5 text-xs text-muted-foreground">
          {t('textFormat.desc')}
        </p>
      </div>
      <div className="divide-y divide-border/60">
        {/* 变量名从 t 改成 def：t 现在是翻译函数，同名会被遮蔽掉 */}
        {TOGGLES.map((def) => (
          <div key={def.key} className="flex items-center gap-2.5 px-4 py-2.5">
            <Switch
              checked={opts[def.key]}
              onChange={() => toggle(def.key)}
              size="sm"
              noAnimation={!animate}
              hidden={!ready}
              className="shrink-0"
            />
            <span className="shrink-0 text-sm font-medium">{t(def.titleKey)}</span>
            <div className="flex min-w-0 flex-1 items-center gap-1.5">
              <span className="min-w-0 truncate text-xs text-muted-foreground">{t(def.hintKey)}</span>
              <Tooltip content={`${t(def.detailKey)}${t('textFormat.aiNote')}`} variant="light">
                <HelpCircle className="h-3.5 w-3.5 shrink-0 cursor-help text-muted-foreground/50 hover:text-muted-foreground" />
              </Tooltip>
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}
