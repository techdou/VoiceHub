import React from 'react'
import ReactDOM from 'react-dom/client'
import { listen } from '@tauri-apps/api/event'
import { initLanguage } from '@/stores/language'
import { initTheme } from '@/stores/theme'
import TrayMenu from './TrayMenu'
import './tray-menu.css'
import '@/index.css'

async function bootstrap() {
  // 这个 WebView 独立于主窗口：每次启动都要自己读回语言和主题，不能依赖主窗口 DOM。
  await Promise.all([initLanguage(), initTheme()])

  ReactDOM.createRoot(document.getElementById('root')!).render(
    <React.StrictMode>
      <TrayMenu />
    </React.StrictMode>,
  )
}

// 禁掉 WebView 自带右键菜单，避免在托盘菜单里再弹出浏览器菜单。
window.addEventListener('contextmenu', (event) => event.preventDefault())

// 主窗口里切换语言/主题后，下一次展开托盘菜单会重新读取设置。
void listen('tray-menu-open', () => {
  void initLanguage()
  void initTheme()
})

void bootstrap()
