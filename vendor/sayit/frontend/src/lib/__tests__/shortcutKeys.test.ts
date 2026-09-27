import { describe, expect, it } from 'vitest'
import {
  canonicalizePTTShortcut,
  displayPTTShortcut,
  getAcceleratorShortcutValidationError,
  getPTTShortcutWarning,
  getPTTShortcutValidationError,
  isValidPTTShortcut,
  keyEventToShortcutCandidate,
  parsePTTShortcut,
  PTT_CODE_TO_VK,
  pttShortcutConflictsWithAccelerator,
  pttShortcutHasModifier,
  pttShortcutToAccelerator,
} from '../shortcutKeys'

describe('PTT 物理组合键', () => {
  it('保留左右位置并按固定顺序规范化', () => {
    const value = canonicalizePTTShortcut('ShiftRight+KeyK+MetaLeft+ControlLeft')
    expect(value).toBe('ControlLeft+MetaLeft+ShiftRight+KeyK')
    expect(parsePTTShortcut(value)).toEqual([
      'ControlLeft',
      'MetaLeft',
      'ShiftRight',
      'KeyK',
    ])
    expect(displayPTTShortcut('ControlLeft+MetaLeft')).toEqual(['左 Ctrl', '左 Win'])
  })

  it('兼容旧单键，并接受普通组合与纯修饰组合', () => {
    // 这里原来用 ShiftRight 当旧单键的样例，现在 Shift 已被 PTT 禁用（见下方专门的用例），
    // 换成同样是"旧单键"的右 Alt / CapsLock
    expect(isValidPTTShortcut('AltRight')).toBe(true)
    expect(isValidPTTShortcut('CapsLock')).toBe(true)
    expect(isValidPTTShortcut('MButton')).toBe(true)
    expect(isValidPTTShortcut('ControlLeft+KeyK')).toBe(true)
    expect(isValidPTTShortcut('ControlLeft+MetaLeft')).toBe(true)
    expect(pttShortcutHasModifier('ControlLeft+KeyK')).toBe(true)
    expect(PTT_CODE_TO_VK.MetaLeft).toBe(0x5b)
    expect(PTT_CODE_TO_VK.KeyK).toBe(0x4b)
  })

  // F13–F24 是给可编程键盘 / 自制 HID 语音设备当专用触发键用的（物理键盘上没有这些键，
  // 因此几乎不与任何程序冲突）。
  //
  // 这条断言与 Rust 的 function_keys_f1_through_f24_are_all_present_with_contiguous_vks
  // 对称：两张表是手写副本，只改一边是静默失败 —— 前端放行 F13、Rust 查不到它，
  // 用户界面上显示 F13，实际说话键会悄悄回落成右 Ctrl。
  it('F1–F24 全部可用，vk 连续且可单独作为按住说话键', () => {
    for (let n = 1; n <= 24; n += 1) {
      const code = `F${n}`
      expect(PTT_CODE_TO_VK[code], `${code} 应当在按键表里`).toBe(0x70 + n - 1)
      expect(isValidPTTShortcut(code), `${code} 应当可以单独当按住说话键`).toBe(true)
    }
    // 语言中立的键名不进 locale，直接用表里的 label
    expect(displayPTTShortcut('F13')).toEqual(['F13'])
    expect(displayPTTShortcut('F24')).toEqual(['F24'])
    // 也能当组合成员；Alt+F4 那条保留组合只匹配 F4，不牵连 F13–F24
    expect(isValidPTTShortcut('ControlLeft+F13')).toBe(true)
    expect(isValidPTTShortcut('AltLeft+F13')).toBe(true)
    expect(getPTTShortcutValidationError('AltLeft+F4')).not.toBeNull()
  })

  it('拒绝单独 Win、裸字母、多主键和危险系统组合', () => {
    expect(getPTTShortcutValidationError('MetaLeft')).toContain('不能单独')
    expect(getPTTShortcutValidationError('KeyK')).toContain('不能单独')
    expect(getPTTShortcutValidationError('ControlLeft+KeyK+KeyL')).toContain('最多')
    expect(getPTTShortcutValidationError('ControlLeft+ControlRight+KeyK')).toContain('左右')
    expect(getPTTShortcutValidationError('MetaLeft+KeyL')).toContain('系统组合')
    expect(getPTTShortcutValidationError('AltLeft+F4')).toContain('系统组合')
    expect(getPTTShortcutValidationError('MetaLeft+KeyK')).not.toBeNull()
    expect(getPTTShortcutValidationError('AltLeft+Space')).not.toBeNull()
    expect(isValidPTTShortcut('ControlLeft+MetaLeft')).toBe(true)
  })

  // 原来是「提示风险但仍允许保存」。警告挡不住任何人：用户照样绑了右 Shift，
  // 照样踩到筛选键让录音停不下来。现在改成硬拦。
  it('按住说话一律拒绝 Shift，单键和组合成员都算', () => {
    expect(isValidPTTShortcut('ShiftRight')).toBe(false)
    expect(isValidPTTShortcut('ShiftLeft')).toBe(false)
    expect(isValidPTTShortcut('ControlLeft+ShiftRight')).toBe(false)
    expect(isValidPTTShortcut('ShiftLeft+KeyK')).toBe(false)
    expect(getPTTShortcutValidationError('ShiftRight')).toContain('Shift')
    // 别的修饰键不受影响
    expect(isValidPTTShortcut('AltRight')).toBe(true)
    expect(isValidPTTShortcut('ControlLeft+KeyK')).toBe(true)
  })

  // 老用户可能在 Shift 还能选的时候绑上了它：旧绑定继续生效，但要在设置里提示改绑。
  // 升级时静默换掉用户的说话键，或者让它突然失效，都比留一行提示更糟。
  it('已保存的 Shift 旧配置给出改绑提示', () => {
    expect(getPTTShortcutWarning('ShiftRight')).toContain('Shift')
    expect(getPTTShortcutWarning('ControlLeft+ShiftLeft')).toContain('Shift')
    expect(getPTTShortcutWarning('AltRight')).toBeNull()
  })

  // Shift 只在「按住说话」里有问题（要长按、会连按）。免提这类按一下的快捷键照旧可用。
  it('按一下的快捷键不受 Shift 限制', () => {
    expect(getAcceleratorShortcutValidationError('CommandOrControl+Shift+K')).toBeNull()
  })

  it('通用组合键同样拒绝 Windows 保留快捷键', () => {
    expect(getAcceleratorShortcutValidationError('Control+Alt+Delete')).not.toBeNull()
    expect(getAcceleratorShortcutValidationError('Alt+Tab')).not.toBeNull()
    expect(getAcceleratorShortcutValidationError('Alt+Space')).not.toBeNull()
    expect(getAcceleratorShortcutValidationError('Control+Shift+Escape')).not.toBeNull()
    expect(getAcceleratorShortcutValidationError('Super+K')).not.toBeNull()
    expect(getAcceleratorShortcutValidationError('CommandOrControl+K')).toBeNull()
  })

  it('可与免提 accelerator 做语义冲突比较', () => {
    expect(pttShortcutToAccelerator('ControlLeft+KeyK')).toBe('CommandOrControl+K')
    expect(
      pttShortcutConflictsWithAccelerator('ControlLeft+KeyK', 'CommandOrControl+K'),
    ).toBe(true)
    expect(pttShortcutConflictsWithAccelerator('ShiftRight', 'ShiftRight')).toBe(true)
    expect(
      pttShortcutConflictsWithAccelerator('ControlLeft+MetaLeft', 'ControlLeft'),
    ).toBe(true)
    expect(
      pttShortcutConflictsWithAccelerator('ControlLeft+MetaLeft', 'CommandOrControl+K'),
    ).toBe(false)
  })
})
describe('按键事件 → 候选快捷键', () => {
  /** 只造出 keyEventToShortcutCandidate 会读的那几个字段 */
  function keyEvent(init: {
    code: string
    key: string
    ctrl?: boolean
    alt?: boolean
    shift?: boolean
    meta?: boolean
  }): KeyboardEvent {
    return {
      code: init.code,
      key: init.key,
      ctrlKey: init.ctrl ?? false,
      altKey: init.alt ?? false,
      shiftKey: init.shift ?? false,
      metaKey: init.meta ?? false,
    } as KeyboardEvent
  }

  // 用户反馈的原始症状：向导里按 Ctrl+D 会被静默存成「左 Ctrl」。
  // 成因是 Ctrl 的 keydown 先到、ControlLeft 恰好在单键白名单里，于是提交了它、
  // D 根本没轮到。修法是把「先组合、后单键」的顺序钉在这里。
  it('Ctrl+D 录成组合键，中途单按 Ctrl 只是尚未成型', () => {
    expect(keyEventToShortcutCandidate(keyEvent({ code: 'ControlLeft', key: 'Control', ctrl: true })))
      .toBe('ControlLeft')
    expect(keyEventToShortcutCandidate(keyEvent({ code: 'KeyD', key: 'd', ctrl: true })))
      .toBe('CommandOrControl+D')
  })

  it('修饰键单键仍然可用（免提默认就是右 Alt）', () => {
    expect(keyEventToShortcutCandidate(keyEvent({ code: 'AltRight', key: 'Alt', alt: true })))
      .toBe('AltRight')
    expect(keyEventToShortcutCandidate(keyEvent({ code: 'ControlRight', key: 'Control', ctrl: true })))
      .toBe('ControlRight')
  })

  // Space / Insert / F1–F24 同时出现在单键白名单里，判定顺序反了就会把
  // Ctrl+Space、Ctrl+F1 也吃成单键 —— 用户以为设了组合键，实际绑的是空格。
  it('同时属于单键白名单的键，带修饰时优先当组合键', () => {
    expect(keyEventToShortcutCandidate(keyEvent({ code: 'Space', key: ' ' }))).toBe('Space')
    expect(keyEventToShortcutCandidate(keyEvent({ code: 'Space', key: ' ', ctrl: true })))
      .toBe('CommandOrControl+Space')
    expect(keyEventToShortcutCandidate(keyEvent({ code: 'F1', key: 'F1' }))).toBe('F1')
    expect(keyEventToShortcutCandidate(keyEvent({ code: 'F1', key: 'F1', ctrl: true })))
      .toBe('CommandOrControl+F1')
  })

  it('裸字母不成型；comboOnly 下单键一律不成型', () => {
    expect(keyEventToShortcutCandidate(keyEvent({ code: 'KeyD', key: 'd' }))).toBeNull()
    expect(keyEventToShortcutCandidate(keyEvent({ code: 'AltRight', key: 'Alt', alt: true }), { comboOnly: true }))
      .toBeNull()
    expect(keyEventToShortcutCandidate(keyEvent({ code: 'F1', key: 'F1' }), { comboOnly: true }))
      .toBeNull()
    expect(
      keyEventToShortcutCandidate(keyEvent({ code: 'KeyD', key: 'd', ctrl: true, shift: true }), { comboOnly: true }),
    ).toBe('CommandOrControl+Shift+D')
  })
})
