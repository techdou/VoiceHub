import { describe, expect, it } from 'vitest'
import {
  cleanActiveMicLabel,
  describeMicSource,
  micSourceChanged,
} from '../micSourceReminder'

describe('microphone source reminder', () => {
  it('describes the actual endpoint selected by system-default mode', () => {
    const source = describeMicSource({
      deviceId: 'default',
      groupId: 'plantronics-group',
      label: 'Default - Headset Microphone (Plantronics Blackwire 5220 Series)',
    }, '', 'Microphone')

    expect(source).toEqual({
      identity: 'auto:group:plantronics-group:headset microphone (plantronics blackwire 5220 series)',
      mode: 'auto',
      label: 'Headset Microphone (Plantronics Blackwire 5220 Series)',
    })
  })

  it('treats fixed and auto routing as different even on the same endpoint', () => {
    const active = {
      deviceId: 'physical-device-1',
      groupId: 'group-1',
      label: 'USB Microphone',
    }

    const automatic = describeMicSource(active, '', 'Microphone')
    const fixed = describeMicSource(active, 'physical-device-1', 'Microphone')

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
})
