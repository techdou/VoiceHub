import { invoke } from '@tauri-apps/api/core'
import { feedRemoteCapture } from '../remoteCapture'

type Packet = { type: 'start'; id: number } | {
  type: 'audio'; id: number; samples: number[]; ended: boolean; error?: string | null
}
export interface RemoteRecorder {
  startRemoteRecording(): Promise<boolean>
  stopRemoteRecording(error?: string | null): Promise<void>
}

export class RemoteTransport {
  private active: number | null = null
  private stopped = false
  private readonly clientId = crypto.randomUUID()
  private timer: ReturnType<typeof setTimeout> | undefined
  constructor(private recorder: RemoteRecorder) {}
  start() { this.stopped = false; void this.tick() }
  stop() {
    this.stopped = true
    clearTimeout(this.timer)
    if (this.active !== null) {
      void this.recorder.stopRemoteRecording('Remote capture stopped').catch(() => {})
      void invoke('remote_voice_reject', { id: this.active }).catch(() => {})
      this.active = null
    }
  }
  private async tick() {
    try {
      const packet = await invoke<Packet | null>('remote_voice_poll', { clientId: this.clientId })
      if (this.stopped) return
      if (packet?.type === 'start') {
        this.active = packet.id
        const ready = await this.recorder.startRemoteRecording()
        if (ready && !this.stopped && await invoke<boolean>('remote_voice_ack', { id: packet.id })) {
          this.active = packet.id
        } else {
          // stop() 在 startRemoteRecording 等待期间执行时（stopped=true），
          // recorder 已被 stop() 停掉：跳过第二次 stop，避免错误消息被
          // 'Remote session expired' 覆盖、也不依赖 stopRemoteRecording 幂等。
          if (ready && !this.stopped) {
            await this.recorder.stopRemoteRecording('Remote session expired')
          }
          // reject 失败不再上抛：掉进外层 catch 会再触发一次 stopRemoteRecording。
          await invoke('remote_voice_reject', { id: packet.id }).catch(() => {})
          this.active = null
        }
      } else if (packet?.type === 'audio') {
        if (packet.id !== this.active) {
          await invoke('remote_voice_reject', { id: packet.id }).catch(() => {})
        } else if (packet.error) {
          await this.recorder.stopRemoteRecording(packet.error)
          await invoke('remote_voice_reject', { id: packet.id }).catch(() => {})
          this.active = null
        } else {
          feedRemoteCapture(packet.samples)
          if (packet.ended) {
            await this.recorder.stopRemoteRecording()
            await invoke('remote_voice_reject', { id: packet.id }).catch(() => {})
            this.active = null
          }
        }
      }
    } catch (error) {
      if (this.active !== null) {
        await this.recorder.stopRemoteRecording(String(error)).catch(() => {})
        await invoke('remote_voice_reject', { id: this.active }).catch(() => {})
        this.active = null
      }
      console.error('[remote-voice]', error)
    } finally {
      if (!this.stopped) this.timer = setTimeout(() => void this.tick(), 25)
    }
  }
}
