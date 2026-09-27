import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  isPseudoInputDevice,
  listMicrophones,
  matchRealEndpoint,
  normalizeSelectedMicId,
  realInputEndpoints,
} from '../audio'

/**
 * `enumerateDevices()` 在 Windows 上返回的形状：一个真实端点，外加 Chromium
 * 自己摆的两个伪设备（default / communications），label 带本地化前缀。
 * 采样自本机日志，不是编的。
 */
const HEADSET = '耳机式麦克风 (Plantronics Blackwire 5220 Series) (047f:c053)'

function device(deviceId: string, label: string, kind = 'audioinput') {
  return { deviceId, groupId: 'plt-group', kind, label, toJSON: () => ({}) }
}

function stubEnumerate(list: ReturnType<typeof device>[]) {
  const getUserMedia = vi.fn(async () => ({ getTracks: () => [] }))
  vi.stubGlobal('navigator', {
    mediaDevices: {
      enumerateDevices: vi.fn(async () => list),
      getUserMedia,
    },
  })
  return getUserMedia
}

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('microphone device list', () => {
  it('recognizes the two pseudo devices Chromium adds, and nothing else', () => {
    expect(isPseudoInputDevice('default')).toBe(true)
    expect(isPseudoInputDevice('communications')).toBe(true)
    expect(isPseudoInputDevice('DEFAULT')).toBe(true)
    expect(isPseudoInputDevice('  ')).toBe(true)
    expect(isPseudoInputDevice('a1b2c3')).toBe(false)
  })

  it('folds a stored pseudo-device id back to "follow system default"', () => {
    // 实测存量数据：`selectedMic = "default"`。老版本的下拉把伪设备也列出来，用户点了它。
    // 不折的话，设置页的下拉找不到选中项，显示成"没有选择麦克风"。
    expect(normalizeSelectedMicId('default')).toBe('')
    expect(normalizeSelectedMicId('communications')).toBe('')
    expect(normalizeSelectedMicId('  ')).toBe('')
    expect(normalizeSelectedMicId(undefined)).toBe('')
    expect(normalizeSelectedMicId(123)).toBe('')
    // 真实 deviceId 一个字都不能动 —— 改了就等于把用户选的设备换掉了。
    expect(normalizeSelectedMicId('real-headset')).toBe('real-headset')
  })

  it('keeps the pseudo devices but drops non-inputs', async () => {
    // 伪设备携带唯一一条「系统默认现在指向谁」的信息，这一层不能替调用方丢掉；
    // 「哪些该进下拉」是产品判断，归 buildMicOptions。
    stubEnumerate([
      device('default', `默认值 - ${HEADSET}`),
      device('communications', `通信设备 - ${HEADSET}`),
      device('real-headset', HEADSET),
      device('speaker', 'Speakers', 'audiooutput'),
    ])

    const mics = await listMicrophones()

    expect(mics.map((d) => d.deviceId)).toEqual(['default', 'communications', 'real-headset'])
  })

  it('resolves which real endpoint the system default currently points at', async () => {
    const devices = [
      { deviceId: 'default', groupId: 'plt-group', label: `默认值 - ${HEADSET}` },
      { deviceId: 'real-headset', groupId: 'plt-group', label: HEADSET },
    ]

    expect(matchRealEndpoint(devices[0], devices)?.deviceId).toBe('real-headset')
    // 真实 deviceId 直接按 id 命中，不走那两层回落。
    expect(matchRealEndpoint(devices[1], devices)?.deviceId).toBe('real-headset')
    // 指向一个已经消失的设备时返回 null，调用方据此退回朴素文案，而不是显示空括号。
    expect(matchRealEndpoint({ deviceId: 'default', groupId: 'gone', label: '默认值 - 没了' }, devices))
      .toBeNull()
  })

  it('treats endpoints without a readable name as not identifiable', async () => {
    const devices = [
      { deviceId: 'a', groupId: 'g', label: '' },
      { deviceId: 'b', groupId: 'g', label: 'Real Mic' },
    ]
    expect(realInputEndpoints(devices).map((d) => d.deviceId)).toEqual(['b'])
  })

  it('still opens a temporary stream when labels are missing, judging by the raw list', async () => {
    // 「拿不到名字」这个信号必须在过滤之前判：伪设备也算数。先过滤会把
    // "只剩一条无名项"这种情况判成"有两条，不用申请权限"。
    const getUserMedia = stubEnumerate([device('', '')])

    await listMicrophones()

    expect(getUserMedia).toHaveBeenCalledTimes(1)
  })
})
