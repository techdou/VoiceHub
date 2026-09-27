import { describe, it, expect } from 'vitest'
import {
  summarizeAppContext,
  buildStatsAppId,
  isModifierPTTSetting,
  computeProcessingTimeoutMs,
  classifyMicLevel,
  judgeOsMicMute,
  hasSilenceEvidence,
  isUnconfirmedPaste,
  SLOW_SEND_INPUT_PASTE_MS,
  MIC_NO_SIGNAL_PEAK_THRESHOLD,
  MIC_LOW_RMS_THRESHOLD,
  OS_MIC_MUTE_CONFIRM_SAMPLES,
} from '../helpers'

describe('hasSilenceEvidence', () => {
  const SILENCE_RMS_THRESHOLD = 0.01

  function stats(peakAmplitude: number, silentFrames: number, totalFrames: number) {
    return { peakAmplitude, silentFrames, totalFrames, silenceRmsThreshold: SILENCE_RMS_THRESHOLD }
  }

  /**
   * 回归钉：判据取「且」不取「或」。
   *
   * 取或的版本会把这条判成"确实没声音"，于是界面告诉用户去检查麦克风，而真正的
   * 失败原因（额度耗尽、服务端提前断开、热词回显被判空）被藏了起来。
   */
  it('说了一句话再长时间沉默，不算没声音', () => {
    expect(hasSilenceEvidence(stats(0.4, 996, 1000))).toBe(false)
  })

  it('整段几乎全是静音帧、峰值也几乎为零，才算有实证', () => {
    expect(hasSilenceEvidence(stats(0.004, 999, 1000))).toBe(true)
  })

  it('峰值够低但静音帧不够多时不下结论', () => {
    expect(hasSilenceEvidence(stats(0.004, 900, 1000))).toBe(false)
  })

  it('一帧都没统计到时不下结论——没有数据不等于没有声音', () => {
    expect(hasSilenceEvidence(stats(0, 0, 0))).toBe(false)
  })

  it('峰值正好等于阈值也不算静音（阈值是严格小于）', () => {
    expect(hasSilenceEvidence(stats(SILENCE_RMS_THRESHOLD, 1000, 1000))).toBe(false)
  })
})

describe('judgeOsMicMute', () => {
  // 回归：Plantronics Blackwire 5220 停在 GetMute=true 但音频照常流动，
  // 直接采信系统标志会导致每次按热键都先误报一次「麦克风已被静音」。
  it('音频在流动时立刻丢弃系统的静音标志', () => {
    expect(judgeOsMicMute(0, 'voiced', 1600)).toEqual({ verdict: 'dismissed' })
    expect(judgeOsMicMute(0, 'low', 1600)).toEqual({ verdict: 'dismissed' })
  })

  it('已攒到快要确认时，一帧非零信号也能推翻它', () => {
    expect(judgeOsMicMute(OS_MIC_MUTE_CONFIRM_SAMPLES - 1, 'low', 1600))
      .toEqual({ verdict: 'dismissed' })
  })

  it('全 0 采样未达阈值时继续攒证据，不弹警告', () => {
    expect(judgeOsMicMute(0, 'muted', 1600)).toEqual({ verdict: 'wait', silentSamples: 1600 })
    expect(judgeOsMicMute(1600, 'muted', 1600)).toEqual({ verdict: 'wait', silentSamples: 3200 })
  })

  it('全 0 采样累计到阈值才确认真被静音', () => {
    expect(judgeOsMicMute(OS_MIC_MUTE_CONFIRM_SAMPLES - 1600, 'muted', 1600))
      .toEqual({ verdict: 'confirmed' })
  })
})

describe('classifyMicLevel', () => {
  it('只有 PCM 峰值严格为 0 才判为 muted', () => {
    expect(classifyMicLevel(0, 0)).toBe('muted')
    expect(classifyMicLevel(0.05, MIC_NO_SIGNAL_PEAK_THRESHOLD)).toBe('muted')
  })

  it('任何非零输入即使极小也判为 low', () => {
    expect(classifyMicLevel(Number.MIN_VALUE, Number.MIN_VALUE)).toBe('low')
    expect(classifyMicLevel(0.004, 0.05)).toBe('low')
    expect(classifyMicLevel(MIC_LOW_RMS_THRESHOLD - 0.0001, 1 / 32768)).toBe('low')
  })

  it('RMS 达到正常水平 判为 voiced', () => {
    expect(classifyMicLevel(0.03, 0.2)).toBe('voiced')
    expect(classifyMicLevel(MIC_LOW_RMS_THRESHOLD, 0.1)).toBe('voiced')
  })
})

describe('summarizeAppContext', () => {
  it('null 返回 null', () => {
    expect(summarizeAppContext(null)).toBeNull()
  })

  it('提取关键字段', () => {
    const result = summarizeAppContext({
      processName: 'code.exe',
      exePath: 'C:\\Program Files\\Code\\code.exe',
      windowTitle: 'secret-doc.md',
      windowClass: 'Chrome_WidgetWin_1',
      focusClass: 'Chrome_RenderWidgetHostHWND',
      controlType: 'Edit',
    })
    expect(result?.processName).toBe('code.exe')
    expect(result?.windowTitle).toBe('secret-doc.md')
  })
})

describe('buildStatsAppId', () => {
  it('优先使用 processName', () => {
    expect(buildStatsAppId({ processName: 'code.exe' } as any)).toBe('code.exe')
  })

  it('processName 为空时用 exePath 最后一段', () => {
    expect(buildStatsAppId({ exePath: 'C:\\Apps\\notepad.exe' } as any)).toBe('notepad.exe')
  })

  it('都为空时用 promptAppId', () => {
    expect(buildStatsAppId(null, 'my-app')).toBe('my-app')
  })

  it('全部为空返回 unknown', () => {
    expect(buildStatsAppId(null)).toBe('unknown')
  })
})

describe('isModifierPTTSetting', () => {
  it('识别单个修饰键和组合中的任意修饰键', () => {
    expect(isModifierPTTSetting('AltLeft')).toBe(true)
    expect(isModifierPTTSetting('ControlRight')).toBe(true)
    expect(isModifierPTTSetting('ShiftLeft')).toBe(true)
    expect(isModifierPTTSetting('ControlLeft+KeyK')).toBe(true)
    expect(isModifierPTTSetting('ControlLeft+MetaLeft')).toBe(true)
  })

  it('非修饰键返回 false', () => {
    expect(isModifierPTTSetting('Space')).toBe(false)
    expect(isModifierPTTSetting('F1')).toBe(false)
    expect(isModifierPTTSetting('KeyK')).toBe(false)
    expect(isModifierPTTSetting(undefined)).toBe(false)
  })
})

describe('computeProcessingTimeoutMs', () => {
  it('server 模式基础超时', () => {
    const ms = computeProcessingTimeoutMs(5, 'server')
    expect(ms).toBeGreaterThanOrEqual(15000)
    expect(ms).toBeLessThan(20000)
  })

  it('cloud_api 模式至少 30s', () => {
    const ms = computeProcessingTimeoutMs(1, 'cloud_api')
    expect(ms).toBeGreaterThanOrEqual(30000)
  })

  it('local 模式至少 30s', () => {
    const ms = computeProcessingTimeoutMs(1, 'local')
    expect(ms).toBeGreaterThanOrEqual(30000)
  })

  it('长音频超时更长', () => {
    const short = computeProcessingTimeoutMs(5, 'server')
    const long = computeProcessingTimeoutMs(60, 'server')
    expect(long).toBeGreaterThan(short)
  })

  it('cloud_api 上限 90s', () => {
    const ms = computeProcessingTimeoutMs(600, 'cloud_api')
    expect(ms).toBeLessThanOrEqual(90000)
  })
})

describe('isUnconfirmedPaste', () => {
  it('send_input 卡住（真实那次 2088ms）要弹兜底卡片', () => {
    expect(isUnconfirmedPaste('send_input', 2088)).toBe(true)
    expect(isUnconfirmedPaste('send_input', SLOW_SEND_INPUT_PASTE_MS)).toBe(true)
  })

  it('正常的 send_input 仍按成功（日志里最慢 631ms）', () => {
    expect(isUnconfirmedPaste('send_input', 631)).toBe(false)
    expect(isUnconfirmedPaste('send_input', SLOW_SEND_INPUT_PASTE_MS - 1)).toBe(false)
  })

  it('会核实结果或同步等目标的方式，慢也不弹', () => {
    expect(isUnconfirmedPaste('wm_paste', 5000)).toBe(false)
    expect(isUnconfirmedPaste('console_paste', 5000)).toBe(false)
    expect(isUnconfirmedPaste(undefined, 5000)).toBe(false)
  })
})
