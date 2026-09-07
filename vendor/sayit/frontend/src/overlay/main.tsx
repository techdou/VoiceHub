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

// overlay-ping 健康检测监听已移除：上游本就没有任何 native 侧发射端
// （连同 overlay_pong 命令一起是未完成机制），监听只会空转。

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <Overlay />
  </React.StrictMode>
)
