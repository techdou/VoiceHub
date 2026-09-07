import { startWebviewKeyboardFallback } from '../services/webviewKeyboardFallback'
import React from 'react'
import ReactDOM from 'react-dom/client'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import Overlay from './Overlay'
import '../index.css'

// Transparent background for overlay window
const style = document.createElement('style')
style.textContent = 'html, body, #root { background: transparent !important; }'
document.head.appendChild(style)

void startWebviewKeyboardFallback()

// Health check: respond to ping from main process so it can detect
// WebView2 unresponsiveness. Bug 003 diagnostic.
void listen<number>('overlay-ping', (event) => {
  const seq = typeof event.payload === 'number' ? event.payload : 0
  void invoke('overlay_pong', { seq }).catch(() => {})
})

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <Overlay />
  </React.StrictMode>
)
