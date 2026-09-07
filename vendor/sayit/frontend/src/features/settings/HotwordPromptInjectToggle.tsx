// 「热词注入提示词」开关卡片
// 开启后，AI 整理阶段会把用户热词表注入系统提示词，帮助纠正/保留专有名词。默认关闭。

import { useEffect, useState } from 'react'
import { Card, CardContent } from '@/components/ui/card'
import { Switch } from '@/components/ui/switch'
import { getSetting, setSetting } from '@/services/store'
import { useT } from '@/i18n/useT'

export default function HotwordPromptInjectToggle() {
  const t = useT()
  const [enabled, setEnabled] = useState(false)
  // 读到已保存值之前，开关先隐藏、且不放动画：避免先画出默认值再跳到已保存值（闪一下）。
  // 用 finally 兜底，读取失败也让开关出现（呈默认值），不至于一直隐藏。
  const [ready, setReady] = useState(false)
  // animate 与 ready 分开：ready 决定何时显示，animate 决定何时允许过渡。
  // 若在揭开/赋值的同一帧就把 transition 加回来，按 CSS 规范浏览器会认为
  // 「有过渡且值变了」，于是把「默认值→已保存值」真的动画一遍（看起来就是闪一下）。
  // 所以揭开那一帧仍不带过渡，隔两帧待值稳定后才开过渡。
  const [animate, setAnimate] = useState(false)

  useEffect(() => {
    getSetting('injectHotwordsToPrompt', false)
      .then((v) => setEnabled(Boolean(v)))
      .finally(() => {
        setReady(true)
        requestAnimationFrame(() => requestAnimationFrame(() => setAnimate(true)))
      })
  }, [])

  const toggle = () => {
    const next = !enabled
    setEnabled(next)
    void setSetting('injectHotwordsToPrompt', next)
  }

  return (
    <Card>
      <CardContent className="p-6">
        <div className="flex items-center justify-between">
          <div className="pr-4">
            <h2 className="text-lg font-semibold">{t('hotwordInject.title')}</h2>
            <p className="mt-1 text-xs text-muted-foreground">
              {t('hotwordInject.desc')}
            </p>
          </div>
          <Switch checked={enabled} onChange={toggle} noAnimation={!animate} hidden={!ready} />
        </div>
      </CardContent>
    </Card>
  )
}
