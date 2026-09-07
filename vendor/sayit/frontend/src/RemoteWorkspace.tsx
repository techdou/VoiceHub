import { useEffect, useRef } from 'react'
import { useParams } from 'react-router-dom'
import { createApp, reactive, h } from 'vue'
import HardwareApp from '../../../../src/App.vue'
import hardwareCss from '../../../../src/styles.css?inline'
import { useLocale } from '@/i18n/useT'

export default function RemoteWorkspace() {
  const { page = 'connection' } = useParams()
  const locale = useLocale() === 'en' ? 'en' : 'zh'
  const host = useRef<HTMLDivElement>(null)
  const state = useRef(reactive<{ embedded: boolean; initialPage: string; locale: 'en' | 'zh' }>({ embedded: true, initialPage: page, locale }))
  useEffect(() => { state.current.initialPage = page }, [page])
  useEffect(() => { state.current.locale = locale }, [locale])
  useEffect(() => {
    const shadow = host.current!.shadowRoot ?? host.current!.attachShadow({ mode: 'open' })
    const style = document.createElement('style')
    // Scope the original hardware tokens and reset to the shadow host.
    style.textContent = hardwareCss.replace(/:root(:not\(\[data-theme="light"\]\)|\[data-theme="(?:light|dark)"\])/g, ':host($1)').replaceAll(':root', ':host').replaceAll('html, body', ':host') +
      '\n:host{display:block;overflow:auto;height:100%;font-size:14px;font-family:inherit;--bg:hsl(var(--background));--text:hsl(var(--foreground));--text-secondary:hsl(var(--muted-foreground));--panel:hsl(var(--background));--panel-2:hsl(var(--muted));--accent:#16745b;--accent-soft:#e4f2ed;--border:hsl(var(--input));color:var(--text)}.app-shell{display:block;height:auto;background:transparent}.main{padding:0}.page{max-width:896px;margin:0 auto;padding:0}.page h1{font-size:23px}.page-sub{display:none}.card{box-shadow:none;border-radius:0;background:transparent;border-width:0 0 1px;padding:20px 0}button,input,select{font:inherit}.device-item,.btn,.picker-item{border-radius:6px}*{letter-spacing:0} .row{flex-wrap:wrap}input,select{max-width:100%}'
    style.textContent += '\n:host{--control:hsl(var(--background));--accent-strong:#105c48;--border-strong:hsl(var(--input));--radius:6px} :host([data-theme="dark"]){--accent:#6bd4b0;--accent-strong:#46ad8c;--accent-soft:#193f33;--control:#292b2a} .btn.primary{color:white} :host([data-theme="dark"]) .btn.primary{color:#10271f} a{color:var(--accent)}'
    const container = document.createElement('div')
    const scoped = document.createElement('style')
    const syncStyles = () => {
      scoped.textContent = Array.from(document.styleSheets).flatMap(sheet => {
        try { return Array.from(sheet.cssRules).filter(rule => rule.cssText.includes('data-v-')).map(rule => rule.cssText) }
        catch { return [] }
      }).join('\n')
      host.current!.setAttribute('data-theme', document.documentElement.classList.contains('dark') ? 'dark' : 'light')
    }
    syncStyles()
    const observer = new MutationObserver(syncStyles)
    observer.observe(document.head, { childList: true, subtree: true, characterData: true })
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] })
    shadow.append(style, scoped, container)
    const app = createApp({ render: () => h(HardwareApp, { ...state.current }) })
    app.mount(container)
    return () => { observer.disconnect(); app.unmount(); shadow.replaceChildren() }
  }, [])
  return <div ref={host} className="h-full min-w-0" />
}
