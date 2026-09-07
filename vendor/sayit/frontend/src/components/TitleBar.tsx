import { useEffect, useState } from 'react'
import * as bridge from '@/services/bridge'
import { Minus, Square, X, Wand2 } from 'lucide-react'
import appIcon from '../assets/voicehub.svg'
import { useAiEnabled, useAiEnabledReady } from '@/hooks/useAiEnabled'
import { useActivePreset } from '@/hooks/useActivePreset'
import { toggleAiEnabled } from '@/stores/aiEnabled'
import { Tooltip } from '@/components/ui/tooltip'
import { useT } from '@/i18n/useT'
import { recordedPromptPresetDisplayName } from '@/i18n/displayNames'

export default function TitleBar() {
  const t = useT()
  const aiEnabled = useAiEnabled()
  // 冷启动时 AI 初始值异步读回：就绪前开关先隐藏、不放动画，避免自己从关跳到开
  const ready = useAiEnabledReady()
  // 同 AppearancePage：显示与「允许过渡」必须错开一帧，否则揭开那一刻会把
  // 开关从默认(关)到已保存(开)真的滑动一遍。
  const [animate, setAnimate] = useState(false)
  useEffect(() => {
    if (!ready || animate) return
    const id = requestAnimationFrame(() => requestAnimationFrame(() => setAnimate(true)))
    return () => cancelAnimationFrame(id)
  }, [ready, animate])
  const activePreset = useActivePreset()
  const presetName = activePreset.name
    ? recordedPromptPresetDisplayName(activePreset.id, activePreset.name)
    : t('titleBar.defaultPreset')

  return (
    <div data-tauri-drag-region className="voicehub-titlebar flex h-12 shrink-0 items-center justify-between bg-titlebar border-b select-none"
      style={{ WebkitAppRegion: 'drag' } as React.CSSProperties}>
      <div data-tauri-drag-region className="flex items-center gap-2.5 pl-5">
        <img src={appIcon} alt="VoiceHub" className="h-7 w-7" draggable={false} />
        <span data-tauri-drag-region className="whitespace-nowrap text-sm font-semibold text-foreground">声枢 <span className="voicehub-brand-english ml-1.5 text-xs font-normal text-muted-foreground">VoiceHub</span></span>
      </div>
      <div className="flex items-center">
        <div className="flex items-center pr-10" style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}>
          <Tooltip content={t('titleBar.presetTooltip', { name: presetName })}>
            <button
              type="button"
              role="switch"
              aria-checked={aiEnabled}
              onClick={() => { void toggleAiEnabled() }}
              className="flex items-center gap-1.5 rounded-md px-2 py-1 transition-colors hover:bg-accent"
              aria-label={t('titleBar.aiCleanupToggle')}
            >
              <Wand2 className={aiEnabled ? 'h-3.5 w-3.5 text-primary' : 'h-3.5 w-3.5 text-muted-foreground'} />
              <span className={`voicehub-ai-label whitespace-nowrap text-xs ${aiEnabled ? 'text-foreground' : 'text-muted-foreground'}`}>{t('titleBar.aiCleanup')}</span>
              <span className={`relative h-4 w-7 shrink-0 rounded-full ${animate ? 'transition-colors' : ''} ${aiEnabled ? 'bg-primary' : 'bg-muted'}`} style={ready ? undefined : { visibility: 'hidden' }}>
                <span className={`absolute left-0.5 top-0.5 h-3 w-3 rounded-full bg-card shadow ${animate ? 'transition-transform' : ''} ${aiEnabled ? 'translate-x-3' : ''}`} />
              </span>
            </button>
          </Tooltip>
        </div>
        <div className="flex" style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}>
          <button onClick={() => bridge.minimize()}
            className="flex h-10 w-11 items-center justify-center hover:bg-accent"
            aria-label={t('window.minimize')}>
            <Minus className="h-4 w-4" />
          </button>
          <button onClick={() => bridge.maximize()}
            className="flex h-10 w-11 items-center justify-center hover:bg-accent"
            aria-label={t('window.maximize')}>
            <Square className="h-3 w-3" />
          </button>
          <button onClick={() => bridge.close()}
            className="flex h-10 w-11 items-center justify-center hover:bg-titlebar-close-hover hover:text-titlebar-close-hover-text"
            aria-label={t('window.close')}>
            <X className="h-4 w-4" />
          </button>
        </div>
      </div>
    </div>
  )
}
