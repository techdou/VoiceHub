import type { ActiveMicrophoneInfo } from '../audio'

export type MicSourceMode = 'auto' | 'fixed'

export interface MicSourceDescriptor {
  identity: string
  mode: MicSourceMode
  label: string
}

/** Keep system-provided friendly names readable while preserving meaningful model suffixes. */
export function cleanActiveMicLabel(label: string): string {
  return label
    .replace(/^\s*(?:default|communications)\s*[-\u2013\u2014:]\s*/i, '')
    .replace(/\s*\([0-9a-f]{4}:[0-9a-f]{4}\)\s*$/i, '')
    .trim()
}

export function describeMicSource(
  active: ActiveMicrophoneInfo,
  requestedDeviceId: string,
  fallbackLabel: string,
): MicSourceDescriptor {
  const mode: MicSourceMode = requestedDeviceId ? 'fixed' : 'auto'
  const label = cleanActiveMicLabel(active.label) || fallbackLabel
  const deviceId = active.deviceId.trim()
  const groupId = active.groupId.trim()

  // deviceId is normally stable for this WebView origin. Some Chromium/Windows
  // combinations expose the literal "default" pseudo-device; in that case the
  // friendly label (and then group id) is the best available physical identity.
  const physicalIdentity = deviceId && deviceId.toLowerCase() !== 'default'
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
