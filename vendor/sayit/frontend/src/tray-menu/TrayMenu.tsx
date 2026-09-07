import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { AppWindow, Power, Wand2 } from 'lucide-react'
import { t } from '@/i18n'
import { useT } from '@/i18n/useT'

type AiStatePayload = { enabled?: boolean }

export default function TrayMenu() {
  useT()
  const [aiEnabled, setAiEnabled] = useState(true)
  const [switching, setSwitching] = useState(false)
  const firstItemRef = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    void invoke<boolean>('get_tray_ai_enabled').then(setAiEnabled)

    const unlistenState = listen<AiStatePayload>('tray-ai-state', (event) => {
      setAiEnabled(Boolean(event.payload?.enabled))
    })
    const unlistenOpen = listen<AiStatePayload>('tray-menu-open', (event) => {
      setAiEnabled(Boolean(event.payload?.enabled))
      requestAnimationFrame(() => firstItemRef.current?.focus())
    })

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') void invoke('hide_tray_menu')
    }
    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
      void unlistenState.then((fn) => fn())
      void unlistenOpen.then((fn) => fn())
    }
  }, [])

  const openMain = () => void invoke('show_main_from_tray')

  const toggleAi = async () => {
    if (switching) return
    setSwitching(true)
    try {
      const next = await invoke<boolean>('toggle_tray_ai_enabled')
      setAiEnabled(next)
    } finally {
      setSwitching(false)
    }
  }

  const quit = () => void invoke('quit_from_tray')

  return (
    <div className="tray-menu-shell" role="menu" aria-label="SayIt">
      <button ref={firstItemRef} className="tray-menu-item" role="menuitem" onClick={openMain}>
        <span className="tray-menu-leading" aria-hidden>
          <AppWindow size={14} strokeWidth={1.8} />
        </span>
        <span>{t('tray.open')}</span>
      </button>

      <button
        className="tray-menu-item"
        role="menuitemcheckbox"
        aria-checked={aiEnabled}
        aria-label={t('tray.aiCleanupStatus', {
          label: t('tray.aiCleanup'),
          state: t(aiEnabled ? 'tray.on' : 'tray.off'),
        })}
        disabled={switching}
        onClick={() => void toggleAi()}
      >
        <span className={`tray-menu-leading tray-menu-ai-icon ${aiEnabled ? 'is-on' : ''}`} aria-hidden>
          <Wand2 size={14} strokeWidth={1.8} />
        </span>
        <span>{t('tray.aiCleanup')}</span>
        <span className={`tray-menu-switch ${aiEnabled ? 'is-on' : 'is-off'}`} aria-hidden>
          <span className="tray-menu-switch-thumb" />
        </span>
      </button>

      <div className="tray-menu-divider" role="separator" />

      <button className="tray-menu-item tray-menu-quit" role="menuitem" onClick={quit}>
        <span className="tray-menu-leading" aria-hidden>
          <Power size={14} strokeWidth={1.8} />
        </span>
        <span>{t('tray.quit')}</span>
      </button>
    </div>
  )
}
