import { describe, expect, it } from 'vitest'
import type { MicEndpoint } from '../../audio'
import { buildMicOptions } from '@/features/settings/utils'
import {
  cleanActiveMicLabel,
  describeMicSource,
  micSourceChanged,
} from '../micSourceReminder'

/** 本机日志里的真实形状：结尾还挂着 USB 的 VID:PID。 */
const HEADSET_RAW = '耳机式麦克风 (Plantronics Blackwire 5220 Series) (047f:c053)'
/** 两处都该显示的这一串：剥掉本地化前缀与 USB 标识，括号里的型号保留。 */
const HEADSET_SHOWN = '耳机式麦克风 (Plantronics Blackwire 5220 Series)'

const headsetSnapshot: MicEndpoint[] = [
  { deviceId: 'default', groupId: 'plt-group', label: `默认值 - ${HEADSET_RAW}` },
  { deviceId: 'communications', groupId: 'plt-group', label: `通信设备 - ${HEADSET_RAW}` },
  { deviceId: 'real-headset', groupId: 'plt-group', label: HEADSET_RAW },
]

function followingSystemDefault(devices = headsetSnapshot) {
  return {
    deviceId: 'default',
    groupId: 'plt-group',
    label: `默认值 - ${HEADSET_RAW}`,
    devices,
  }
}

describe('microphone source reminder', () => {
  it('describes the actual endpoint selected by system-default mode', () => {
    const source = describeMicSource({
      deviceId: 'default',
      groupId: 'plantronics-group',
      label: 'Default - Headset Microphone (Plantronics Blackwire 5220 Series)',
      devices: [],
    }, '', 'Microphone')

    expect(source).toEqual({
      identity: 'auto:group:plantronics-group:headset microphone (plantronics blackwire 5220 series)',
      mode: 'auto',
      // 拿不到快照时只能靠字面清理剥前缀；括号里的型号照样保留。
      label: 'Headset Microphone (Plantronics Blackwire 5220 Series)',
    })
  })

  it('treats fixed and auto routing as different even on the same endpoint', () => {
    const endpoint = {
      deviceId: 'physical-device-1',
      groupId: 'group-1',
      label: 'USB Microphone',
      devices: [],
    }

    const automatic = describeMicSource(endpoint, '', 'Microphone')
    const fixed = describeMicSource(endpoint, 'physical-device-1', 'Microphone')

    expect(automatic.identity).toBe('auto:device:physical-device-1')
    expect(fixed.identity).toBe('fixed:device:physical-device-1')
    expect(micSourceChanged(automatic.identity, fixed.identity)).toBe(true)
  })

  it('only reminds again when the input route changes', () => {
    expect(micSourceChanged(null, 'auto:device:a')).toBe(true)
    expect(micSourceChanged('auto:device:a', 'auto:device:a')).toBe(false)
    expect(micSourceChanged('auto:device:a', 'auto:device:b')).toBe(true)
    expect(micSourceChanged('auto:device:b', 'auto:device:a')).toBe(true)
  })

  it('cleans only technical label noise and keeps model names intact', () => {
    expect(cleanActiveMicLabel('Communications: Studio Mic (1234:abcd)'))
      .toBe('Studio Mic')
    expect(cleanActiveMicLabel('Mic (Plantronics Blackwire 5220 Series)'))
      .toBe('Mic (Plantronics Blackwire 5220 Series)')
  })

  describe('the overlay hint and the settings dropdown must agree', () => {
    it('shows the exact same string as the settings dropdown', () => {
      // 同一个设备在两处叫不同的名字，用户得先在脑子里对一遍才能确认录音走的是他选的
      // 那个 —— 这条断言就是为了把两边钉在一起，改任何一边都会在这里失败。
      const fromOverlay = describeMicSource(followingSystemDefault(), '', 'Microphone').label
      const fromSettings = buildMicOptions(headsetSnapshot, '', {
        systemDefault: '系统默认',
        systemDefaultWith: (device) => `系统默认（${device}）`,
        unnamed: (idPrefix) => `麦克风 ${idPrefix}`,
        unavailable: '上次选的麦克风当前不可用',
      }).find((option) => option.value === 'real-headset')?.label

      expect(fromOverlay).toBe(HEADSET_SHOWN)
      expect(fromSettings).toBe(HEADSET_SHOWN)
      expect(fromOverlay).toBe(fromSettings)
    })

    it('drops the localized pseudo-device prefix without matching it by text', () => {
      // 「默认值 - 」剥掉不是靠正则认中文，而是靠 groupId 找到那个真实端点。
      expect(describeMicSource(followingSystemDefault(), '', 'Microphone').label)
        .toBe(HEADSET_SHOWN)
    })

    it('drops the trailing USB vid:pid, which identifies nothing', () => {
      // 同型号两台设备的 VID:PID 完全一样，留着只占宽度。
      const label = describeMicSource(followingSystemDefault(), '', 'Microphone').label
      expect(label).not.toContain('047f')
    })

    it('falls back to suffix matching when the pseudo device carries no group id', () => {
      const snapshot: MicEndpoint[] = [
        { deviceId: 'default', groupId: '', label: `默认值 - ${HEADSET_RAW}` },
        { deviceId: 'real-headset', groupId: 'plt-group', label: HEADSET_RAW },
      ]
      const source = describeMicSource(
        { ...followingSystemDefault(snapshot), groupId: '' },
        '',
        'Microphone',
      )
      expect(source.label).toBe(HEADSET_SHOWN)
      // 解析到真实端点之后 identity 就是稳定的物理标识，不再掺 label ——
      // 否则系统在 eConsole / eCommunications 之间切默认设备就会被当成换了麦克风。
      expect(source.identity).toBe('auto:device:real-headset')
    })

    it('keeps the parent device name even when it is a sound-card driver name', () => {
      // issue #72 那台机器。这一串长到会被截断，但两处一致比短更重要。
      const array = 'Microphone Array (2- Intel(R) Smart Sound Technology for Digital Microphones)'
      const snapshot: MicEndpoint[] = [
        { deviceId: 'real-array', groupId: 'intel-group', label: array },
      ]
      expect(describeMicSource(
        { deviceId: 'real-array', groupId: 'intel-group', label: array, devices: snapshot },
        'real-array',
        'Microphone',
      ).label).toBe(array)
    })

    it('never returns an empty label, whatever the system reports', () => {
      expect(describeMicSource(
        { deviceId: '', groupId: '', label: '', devices: [] },
        '',
        '麦克风',
      ).label).toBe('麦克风')
    })

    it('still strips the usb id when the device snapshot is unavailable', () => {
      expect(describeMicSource(
        { deviceId: 'real-headset', groupId: 'plt-group', label: HEADSET_RAW, devices: [] },
        'real-headset',
        'Microphone',
      ).label).toBe(HEADSET_SHOWN)
    })
  })
})
