import { describe, expect, it, vi } from 'vitest'
import { createApp, h, nextTick, reactive } from 'vue'
import ConnectionPage from './pages/ConnectionPage.vue'
import type { AppSettings } from './types'

vi.mock('./api', () => ({ api: {
  listPairedRemotes: async () => [], listAudioEndpoints: async () => [],
} }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))

describe('connection settings draft', () => {
  it('renders reactive settings after saving and keeps edits isolated', async () => {
    const settings = reactive({
      schemaVersion: 2, onboardingComplete: true, pairedDeviceId: null,
      pairedDeviceName: null, audioEndpointName: '', gainDb: 0,
      provider: { kind: 'sayit', customVk: 0, customModifiers: 0, customMode: 'hold',
        sayitVk: 0, stopDelayMs: 100, startupGraceMs: 0 },
      profiles: { profiles: [], selectedProfileId: '', smartEnabled: false, rules: { processBindings: {}, fallbackProfileId: '' } },
      buttonMappingEnabled: true, launchAtLogin: false, language: 'system', theme: 'system',
    } as AppSettings)
    const errors: unknown[] = []
    const container = document.createElement('div')
    const app = createApp({ render: () => h(ConnectionPage, {
      settings, bleSnapshot: null, saveState: 'idle', saveError: '',
    }) })
    app.config.errorHandler = error => errors.push(error)
    app.mount(container)
    await nextTick()
    expect(errors).toEqual([])
    expect(container.querySelector('h1')).not.toBeNull()
    const gain = container.querySelector<HTMLInputElement>('input[type="range"]')!
    expect(gain).not.toBeNull()
    gain.value = '6'
    gain.dispatchEvent(new Event('input'))
    gain.dispatchEvent(new Event('change'))
    await nextTick()
    expect(settings.gainDb).toBe(0)
    app.unmount()
  })
})
