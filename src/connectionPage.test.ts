import { describe, expect, it, vi } from 'vitest'
import { createApp, h, nextTick, reactive } from 'vue'
import ConnectionPage from './pages/ConnectionPage.vue'
import type { AppSettings } from './types'

vi.mock('./api', () => ({ api: {
  listPairedRemotes: async () => [], listAudioEndpoints: async () => [],
  // CableInstaller 挂载即轮询虚拟声卡状态；mock 缺这个方法会产生 unhandled rejection。
  checkVirtualCable: async () => ({ endpointPresent: false, servicePresent: false, installState: null, busy: false }),
} }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))

describe('connection settings draft', () => {
  it('renders reactive settings after saving and keeps edits isolated', async () => {
    const settings = reactive({
      schemaVersion: 2, onboardingComplete: true, pairedDeviceId: null,
      pairedDeviceName: null, audioEndpointName: '', gainDb: 0,
      provider: { kind: 'sayit', customVk: 0, customModifiers: 0, customMode: 'hold',
        sayitVk: 0, sayitModifiers: 0, stopDelayMs: 100, startupGraceMs: 0 },
      mapping: { bindings: {} },
      buttonMappingEnabled: true, experimentalVoiceExtend: false, voiceKeyTriggerMode: "ptt", f5GateEnabled: true, launchAtLogin: false, language: 'system', theme: 'system',
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

  // 回归：连接遥控器（后端写盘后 sync-settings 回传新设置）不能把用户未保存的编辑冲掉。
  it('merges only pairing fields when settings update arrives during dirty draft', async () => {
    const settings = reactive({
      schemaVersion: 2, onboardingComplete: true, pairedDeviceId: null,
      pairedDeviceName: null, audioEndpointName: '', gainDb: 0,
      provider: { kind: 'custom', customVk: 0, customModifiers: 0, customMode: 'hold',
        sayitVk: 0, sayitModifiers: 0, stopDelayMs: 100, startupGraceMs: 0 },
      mapping: { bindings: {} },
      buttonMappingEnabled: true, experimentalVoiceExtend: false, voiceKeyTriggerMode: "ptt", f5GateEnabled: true, launchAtLogin: false, language: 'system', theme: 'system',
    } as AppSettings)
    const container = document.createElement('div')
    const app = createApp({ render: () => h(ConnectionPage, {
      settings, bleSnapshot: null, saveState: 'idle', saveError: '',
    }) })
    app.mount(container)
    await nextTick()

    // 用户编辑：切换 provider（未保存）→ dirty 标记出现。
    const sayitOption = [...container.querySelectorAll('.picker-item')][0] as HTMLElement
    sayitOption.click()
    await nextTick()
    const dirtyMark = () => container.querySelector('.dirty-mark')
    expect(dirtyMark()).not.toBeNull()

    // 后端连接完成：settings 整体被替换（含配对字段变化）。
    // 草稿若被整体重置（bug），dirty 标记会消失；正确行为是合并配对字段、保留编辑。
    settings.pairedDeviceId = 'dev-1'
    settings.pairedDeviceName = 'Remote'
    settings.onboardingComplete = true
    await nextTick()
    await nextTick()
    expect(dirtyMark()).not.toBeNull()
    app.unmount()
  })

  // 录音键模式切换（语音触发收敛到录音键）：进草稿、随保存提交，
  // 不能绕过草稿直接写设置。
  it('voice key mode switch goes through the draft and is submitted on save', async () => {
    const settings = reactive({
      schemaVersion: 2, onboardingComplete: true, pairedDeviceId: null,
      pairedDeviceName: null, audioEndpointName: '', gainDb: 0,
      provider: { kind: 'sayit', customVk: 0, customModifiers: 0, customMode: 'hold',
        sayitVk: 0, sayitModifiers: 0, stopDelayMs: 100, startupGraceMs: 0 },
      mapping: { bindings: {} },
      buttonMappingEnabled: true, experimentalVoiceExtend: false, voiceKeyTriggerMode: 'ptt', f5GateEnabled: true,
      launchAtLogin: false, language: 'system', theme: 'system',
    } as AppSettings)
    const saved: AppSettings[] = []
    const container = document.createElement('div')
    const app = createApp({
      render: () => h(ConnectionPage, {
        settings, bleSnapshot: null, saveState: 'idle', saveError: '',
        'onUpdate-settings': (next: AppSettings) => saved.push(next),
      }),
    })
    app.mount(container)
    await nextTick()

    // 切到免提：未保存前原设置不变。
    const buttons = [...container.querySelectorAll('button')] as HTMLButtonElement[]
    const handsFree = buttons.find(b => b.textContent?.includes('免提'))
    expect(handsFree).toBeTruthy()
    handsFree!.click()
    await nextTick()
    expect(settings.voiceKeyTriggerMode).toBe('ptt')

    // 保存：提交的载荷携带新模式。
    const save = buttons.find(b => b.textContent === '保存')
    expect(save).toBeTruthy()
    save!.click()
    await nextTick()
    expect(saved).toHaveLength(1)
    expect(saved[0].voiceKeyTriggerMode).toBe('hands_free')
    app.unmount()
  })
})
