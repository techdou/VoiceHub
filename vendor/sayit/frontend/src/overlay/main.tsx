import { startWebviewKeyboardFallback } from '../services/webviewKeyboardFallback'
import React from 'react'
import ReactDOM from 'react-dom/client'
import Overlay from './Overlay'
import '../index.css'

// Transparent background for overlay window
const style = document.createElement('style')
style.textContent = 'html, body, #root { background: transparent !important; }'
document.head.appendChild(style)

void startWebviewKeyboardFallback()

// VoiceHub：上游的 overlay-ping 健康检查监听已随 dead-listeners-removed 移除——
// 我们 vendor 的 native 侧没有发射端，留着只是死监听。

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <Overlay />
  </React.StrictMode>
)
