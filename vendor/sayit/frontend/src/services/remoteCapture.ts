// Only the active, acknowledged remote generation may feed the recorder callbacks.
export const REMOTE_MIC_ID = 'voicehub-remote-pcm'
let capture: { onData: (data: ArrayBuffer) => void; onFrame?: (pcm: Int16Array) => void } | null = null

export function openRemoteCapture(onData: (data: ArrayBuffer) => void, onFrame?: (pcm: Int16Array) => void) {
  capture = { onData, onFrame }
  return { deviceId: REMOTE_MIC_ID, groupId: 'voicehub', label: 'VoiceHub Remote' }
}
export function feedRemoteCapture(samples: number[]) {
  if (!capture || samples.length === 0) return
  const pcm = Int16Array.from(samples)
  capture.onFrame?.(pcm)
  capture.onData(pcm.buffer)
}
export function closeRemoteCapture() { capture = null }
