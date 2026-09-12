import { describe, expect, it, vi } from 'vitest'
import { createElement } from 'react'
import { createRoot } from 'react-dom/client'
import { createMemoryRouter, RouterProvider } from 'react-router-dom'
import { nextTick } from 'vue'

// 复现 bug：/remote/xxx → /remote/buttons 同路由参数切换时按键映射页空白；
// 而从语音配置（其他路由）直达 /remote/buttons 正常。
// RemoteWorkspace 是唯一同时持有 React Router 参数与 Vue 响应式桥的组件。

vi.mock('./api', () => ({
  api: {
    getSettings: async () => ({
      schemaVersion: 2, onboardingComplete: true, pairedDeviceId: null,
      pairedDeviceName: null, audioEndpointName: '', gainDb: 0,
      provider: { kind: 'sayit', customVk: 0, customModifiers: 0, customMode: 'hold',
        sayitVk: 0, sayitModifiers: 0, stopDelayMs: 100, startupGraceMs: 0 },
      mapping: { bindings: {} },
      buttonMappingEnabled: true, experimentalVoiceExtend: false, voiceKeyTriggerMode: "ptt", launchAtLogin: false, language: 'system', theme: 'system',
    }),
    getBleSnapshot: async () => ({ phase: 'idle' }),
    getStatistics: async () => ({ days: {} }),
    listPairedRemotes: async () => [],
    listAudioEndpoints: async () => [],
    checkVirtualCable: async () => ({ endpointPresent: false, servicePresent: false, installState: null, busy: false }),
  },
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))

// 单方案化后映射画布恒渲染；jsdom 无 ResizeObserver，挂载前补桩（同 buttonsPage.test）。
class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
globalThis.ResizeObserver = (globalThis.ResizeObserver ?? ResizeObserverStub) as typeof ResizeObserver

import RemoteWorkspace from '../vendor/sayit/frontend/src/RemoteWorkspace'

const flush = () => new Promise((resolve) => setTimeout(resolve, 80))

function shadowText(container: HTMLElement): string {
  const host = container.querySelector('div')
  const root = host?.shadowRoot
  return root ? root.textContent ?? '' : '(no shadow root)'
}

function renderRemote(initialPath: string) {
  const container = document.createElement('div')
  document.body.appendChild(container)
  const router = createMemoryRouter(
    [{ path: '/remote/:page', element: createElement(RemoteWorkspace) }],
    { initialEntries: [initialPath] },
  )
  const root = createRoot(container)
  root.render(createElement(RouterProvider, { router }))
  return { container, router, root }
}

describe('RemoteWorkspace page switching', () => {
  it('A: mounting directly at /remote/buttons renders the buttons page', async () => {
    const { container, root } = renderRemote('/remote/buttons')
    await flush()
    await nextTick()
    const text = shadowText(container)
    root.unmount()
    console.log('[A] direct mount text head:', text.slice(0, 80))
    expect(text).toContain('按键映射')
  })

  it('B: navigating /remote/stats -> /remote/buttons renders the buttons page', async () => {
    const { container, router, root } = renderRemote('/remote/stats')
    await flush()
    await nextTick()
    const before = shadowText(container)
    console.log('[B] stats page text head:', before.slice(0, 80))
    expect(before).toContain('使用统计')

    await router.navigate('/remote/buttons')
    await flush()
    await nextTick()
    const after = shadowText(container)
    root.unmount()
    console.log('[B] after navigate text head:', after.slice(0, 80))
    expect(after).toContain('按键映射')
  })
})
