import { isPseudoInputDevice, matchRealEndpoint, stripUsbIds } from '../audio'
import type { ActiveMicrophoneInfo, MicEndpoint } from '../audio'

export type MicSourceMode = 'auto' | 'fixed'

export interface MicSourceDescriptor {
  identity: string
  mode: MicSourceMode
  label: string
}

/**
 * 字面清理，**只在拿不到设备快照时兜底**。
 *
 * 它只认英文前缀，所以中文系统上剥不掉「默认值 - 」—— 那个前缀是 Chromium 按界面
 * 语言生成的，按字面匹配等于给每种语言维护一份，Windows 改写法还会静默失效。
 * 正路是 matchRealEndpoint：直接去快照里取那个不带前缀的真实端点。
 */
export function cleanActiveMicLabel(label: string): string {
  return stripUsbIds(label.replace(/^\s*(?:default|communications)\s*[-\u2013\u2014:]\s*/i, ''))
}

export function describeMicSource(
  active: ActiveMicrophoneInfo,
  requestedDeviceId: string,
  fallbackLabel: string,
  devices: MicEndpoint[] = active.devices ?? [],
): MicSourceDescriptor {
  const mode: MicSourceMode = requestedDeviceId ? 'fixed' : 'auto'
  const real = matchRealEndpoint(active, devices)

  // 显示名与设置页的下拉**必须是同一串**：同一个设备在两处叫不同的名字，用户得先
  // 在脑子里对一遍才能确认录音走的是他选的那个，这比名字长更费劲。所以这里只做
  // 设置页也做的两件事 —— 解析掉本地化前缀（matchRealEndpoint）、剥掉 USB 标识
  // （stripUsbIds），括号里的型号一律保留。
  //
  // 曾经在这里按「同台机器上有没有别人同名」折掉括号里的型号，没有保留：它让两处的
  // 名字不一致，而省下的宽度换不来这个代价。悬浮窗那行放不下时截断即可。
  //
  // 拿不到设备快照（枚举失败）时退回 cleanActiveMicLabel 兜底 —— 它只认英文前缀，
  // 中文系统上剥不掉「默认值 - 」，所以只是兜底，不是正路。
  const label = stripUsbIds(real?.label ?? '') || cleanActiveMicLabel(active.label) || fallbackLabel

  const deviceId = real?.deviceId.trim() || active.deviceId.trim()
  const groupId = real?.groupId.trim() || active.groupId.trim()

  // 解析到真实端点时 deviceId 是稳定的物理标识。解析不到才退回 groupId / 名字 ——
  // 那两条都掺了 label，而伪设备的 label 会随系统在 eConsole / eCommunications 之间
  // 切默认设备而变（「默认值 -」↔「通信设备 -」），于是同一个麦克风被当成换了设备。
  const physicalIdentity = !isPseudoInputDevice(deviceId)
    ? `device:${deviceId}`
    : groupId
      ? `group:${groupId}:${label.toLocaleLowerCase()}`
      : `label:${label.toLocaleLowerCase()}`

  // The routing mode is intentional user-facing information. Switching between
  // auto-detect and a fixed endpoint should be confirmed even if both resolve to
  // the same physical microphone.
  return {
    identity: `${mode}:${physicalIdentity}`,
    mode,
    label,
  }
}

export function micSourceChanged(previousIdentity: string | null, nextIdentity: string): boolean {
  return previousIdentity !== nextIdentity
}
