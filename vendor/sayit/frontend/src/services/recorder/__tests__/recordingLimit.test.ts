import { describe, it, expect } from 'vitest'
import {
  formatRecordingLimit,
  formatRecordingTimer,
  MAX_RECORDING_SEC,
  RECORDING_COUNTDOWN_SEC,
} from '../types'

describe('formatRecordingTimer', () => {
  it('上限之前显示已录秒数', () => {
    expect(formatRecordingTimer(0)).toMatchObject({ text: '0s', countdown: false })
    expect(formatRecordingTimer(12.7)).toMatchObject({ text: '12s', countdown: false })
    // 距上限还剩 61 秒，还没进入倒计时
    expect(formatRecordingTimer(239)).toMatchObject({ text: '239s', countdown: false })
  })

  it('最后一分钟改为显示剩余时间', () => {
    // 正好剩 60 秒时进入倒计时
    expect(formatRecordingTimer(240)).toMatchObject({ text: '剩余 60s', countdown: true, remainingSec: 60 })
    expect(formatRecordingTimer(253)).toMatchObject({ text: '剩余 47s', countdown: true, remainingSec: 47 })
    expect(formatRecordingTimer(299)).toMatchObject({ text: '剩余 1s', countdown: true, remainingSec: 1 })
  })

  it('到点及越界都停在 0，不出现负数', () => {
    expect(formatRecordingTimer(300)).toMatchObject({ text: '剩余 0s', remainingSec: 0 })
    expect(formatRecordingTimer(999)).toMatchObject({ text: '剩余 0s', remainingSec: 0 })
  })

  it('异常输入不炸', () => {
    expect(formatRecordingTimer(-5)).toMatchObject({ text: '0s', countdown: false })
    expect(formatRecordingTimer(Number.NaN)).toMatchObject({ text: '0s', countdown: false })
  })

  it('倒计时窗口必须小于上限（否则一开录就在倒计时）', () => {
    expect(RECORDING_COUNTDOWN_SEC).toBeGreaterThan(0)
    expect(RECORDING_COUNTDOWN_SEC).toBeLessThan(MAX_RECORDING_SEC)
  })

  it('上限必须与 Rust 侧的硬释放时间一致（改一处忘另一处会让提示与实际不符）', () => {
    // keyboard/mod.rs: const HARD_RELEASE_AFTER_SECS: u64 = 5 * 60
    expect(MAX_RECORDING_SEC).toBe(5 * 60)
  })
})

describe('formatRecordingLimit', () => {
  it('文案随上限自动变化（避免改了上限忘了改文案）', () => {
    // 当前上限的中文说明；界面文案直接用它渲染
    expect(formatRecordingLimit()).toBe(
      MAX_RECORDING_SEC < 60
        ? `${MAX_RECORDING_SEC} 秒`
        : MAX_RECORDING_SEC % 60 === 0
          ? `${MAX_RECORDING_SEC / 60} 分钟`
          : `${Math.floor(MAX_RECORDING_SEC / 60)} 分 ${MAX_RECORDING_SEC % 60} 秒`,
    )
  })
})
