import { describe, expect, it, vi } from 'vitest'
import { createApp, h, nextTick, ref } from 'vue'
import ButtonsPage from './pages/ButtonsPage.vue'
import type { AppSettings } from './types'

vi.mock('./api', () => ({ api: {} }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))

// jsdom 没有 ResizeObserver（画布缩放用）；挂载前补桩。
class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
globalThis.ResizeObserver = (globalThis.ResizeObserver ?? ResizeObserverStub) as typeof ResizeObserver

function settingsFixture(): AppSettings {
  return {
    schemaVersion: 2, onboardingComplete: true, pairedDeviceId: null,
    pairedDeviceName: null, audioEndpointName: '', gainDb: 0,
    provider: { kind: 'sayit', customVk: 0, customModifiers: 0, customMode: 'hold',
      sayitVk: 0, sayitModifiers: 0, stopDelayMs: 100, startupGraceMs: 0 },
    mapping: { bindings: {} },
    buttonMappingEnabled: true, experimentalVoiceExtend: false, voiceKeyTriggerMode: 'ptt',
    f5GateEnabled: true, launchAtLogin: false, language: 'system', theme: 'system',
  } as AppSettings
}

function mount(settings: AppSettings, emitted: AppSettings[]) {
  const container = document.createElement('div')
  document.body.appendChild(container)
  const app = createApp({
    render: () => h(ButtonsPage, {
      settings,
      saveState: 'idle', saveError: '',
      'onUpdate-settings': (next: AppSettings) => emitted.push(next),
    }),
  })
  app.mount(container)
  return { container, app }
}

describe('buttons page instant-save UX', () => {
  // 豆哥的核心路径：音量减双击槽绑"删除整行"，选完立即保存生效——
  // 不再存在"草稿 + 保存按钮"这一步（此前用户不知道要保存，配了不生效）。
  it('picking delete-line on the volume-down double slot emits immediately', async () => {
    const settings = settingsFixture()
    const emitted: AppSettings[] = []
    const { container, app } = mount(settings, emitted)

    // 选中音量减卡片。
    const card = [...container.querySelectorAll('.mc-card')]
      .find(el => el.textContent?.includes('音量减键'))
    expect(card).toBeTruthy()
    card!.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await nextTick()

    // 双击槽可点（音量键已放开 secondary），打开动作选择器。
    const slot = [...card!.querySelectorAll('.mc-slot')]
      .find(el => el.textContent?.includes('双击')) as HTMLButtonElement
    expect(slot.disabled).toBe(false)
    slot.click()
    await nextTick()
    expect(container.querySelector('.action-picker-overlay')).not.toBeNull()

    // 选"删除整行"。
    const item = [...container.querySelectorAll('.picker-item')]
      .find(el => el.textContent?.includes('删除整行'))
    expect(item).toBeTruthy()
    item!.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await nextTick()

    // 即时保存：emit 一次，载荷里 volume_down 双击槽 = delete_line。
    expect(emitted).toHaveLength(1)
    const binding = emitted[0].mapping.bindings.volume_down
    expect(binding.double).toEqual({ kind: 'delete_line' })
    // 原设置对象不被就地污染（写穿走 emit，由宿主乐观更新）。
    expect(settings.mapping.bindings.volume_down).toBeUndefined()
    app.unmount()
  })

  // 回归：ABSENT_BUTTONS 必须是响应式的——启动时 bleSnapshot 未回（remoteModel=null
  // 算出空集），遥控器后连上（RC003）置灰要随之出现；absent 键不可选中编辑。
  it('absent markers appear reactively after the remote connects and block editing', async () => {
    const settings = settingsFixture()
    const emitted: AppSettings[] = []
    const model = ref<string | null>(null)
    const container = document.createElement('div')
    document.body.appendChild(container)
    const app = createApp({
      render: () => h(ButtonsPage, {
        settings,
        remoteModel: model.value,
        saveState: 'idle', saveError: '',
        'onUpdate-settings': (next: AppSettings) => emitted.push(next),
      }),
    })
    app.mount(container)
    await nextTick()

    const powerCard = () => [...container.querySelectorAll('.mc-card')]
      .find(el => el.textContent?.includes('电源键'))!
    // 未连接（型号未知）：宁可多显示，不置灰。
    expect(powerCard()).toBeTruthy()
    expect(powerCard().classList.contains('absent')).toBe(false)

    // 遥控器连上、识别为 RC003：电源/返回/TV 置灰，且点击不再进入编辑态。
    model.value = 'Mi Remote Control 2 Pro RC003'
    await nextTick()
    await nextTick()
    expect(powerCard().classList.contains('absent')).toBe(true)
    powerCard().dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await nextTick()
    // 编辑区（按住说话行）只对已选中的非缺席键渲染。
    const holdRow = container.querySelector('.setting-row .label')
    expect(holdRow?.textContent).not.toBe('按住说话')

    // 可用键（音量减）不受影响：仍可选中并出现编辑区。
    const volumeCard = [...container.querySelectorAll('.mc-card')]
      .find(el => el.textContent?.includes('音量减键'))!
    volumeCard.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await nextTick()
    const labels = [...container.querySelectorAll('.setting-row .label')].map(el => el.textContent)
    expect(labels).toContain('按住说话')
    app.unmount()
  })

  it('toggling the master switch emits immediately', async () => {
    const settings = settingsFixture()
    const emitted: AppSettings[] = []
    const { container, app } = mount(settings, emitted)
    const switches = [...container.querySelectorAll('.switch')] as HTMLButtonElement[]
    const master = switches.find(b => b.getAttribute('aria-label')?.includes('按键映射'))
    expect(master).toBeTruthy()
    master!.click()
    await nextTick()
    expect(emitted).toHaveLength(1)
    expect(emitted[0].buttonMappingEnabled).toBe(false)
    app.unmount()
  })
})
