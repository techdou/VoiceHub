import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { feedRemoteCapture } from '../../remoteCapture'
import { RemoteTransport } from '../RemoteTransport'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('../../remoteCapture', () => ({ feedRemoteCapture: vi.fn() }))

describe('remote PCM transport', () => {
  let transport: RemoteTransport
  let packets: unknown[]
  const recorder = { startRemoteRecording: vi.fn(), stopRemoteRecording: vi.fn() }
  beforeEach(() => {
    vi.useFakeTimers(); vi.clearAllMocks(); packets = []
    recorder.startRemoteRecording.mockResolvedValue(true)
    recorder.stopRemoteRecording.mockResolvedValue(undefined)
    vi.mocked(invoke).mockImplementation(async command => command === 'remote_voice_poll' ? packets.shift() ?? null : true)
    transport = new RemoteTransport(recorder)
  })
  afterEach(() => { transport.stop(); vi.useRealTimers() })
  it('waits for capture readiness, then feeds the final batch before stopping', async () => {
    let ready!: (value: boolean) => void
    recorder.startRemoteRecording.mockReturnValue(new Promise(resolve => { ready = resolve }))
    packets.push({ type: 'start', id: 1 }, { type: 'audio', id: 1, samples: [11, 22], ended: true })
    transport.start(); await vi.advanceTimersByTimeAsync(100)
    expect(invoke).not.toHaveBeenCalledWith('remote_voice_ack', expect.anything())
    expect(feedRemoteCapture).not.toHaveBeenCalled()
    ready(true); await vi.advanceTimersByTimeAsync(30)
    expect(feedRemoteCapture).toHaveBeenCalledWith([11, 22])
    expect(recorder.stopRemoteRecording).toHaveBeenCalledWith()
    expect(vi.mocked(feedRemoteCapture).mock.invocationCallOrder[0]).toBeLessThan(recorder.stopRemoteRecording.mock.invocationCallOrder[0])
  })
  it('rejects a busy recorder without acknowledging capture', async () => {
    recorder.startRemoteRecording.mockResolvedValue(false)
    packets.push({ type: 'start', id: 2 }); transport.start()
    await vi.advanceTimersByTimeAsync(30)
    expect(invoke).toHaveBeenCalledWith('remote_voice_reject', { id: 2 })
    expect(invoke).not.toHaveBeenCalledWith('remote_voice_ack', expect.anything())
  })
  it('cancels disconnected capture and discards stale audio', async () => {
    packets.push({ type: 'start', id: 3 }, { type: 'audio', id: 9, samples: [9], ended: true },
      { type: 'audio', id: 3, samples: [], ended: true, error: 'disconnected' })
    transport.start(); await vi.advanceTimersByTimeAsync(80)
    expect(feedRemoteCapture).not.toHaveBeenCalled()
    expect(recorder.stopRemoteRecording).toHaveBeenCalledWith('disconnected')
  })
  it('releases a session if initialization throws', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    recorder.startRemoteRecording.mockRejectedValue(new Error('setup failed'))
    packets.push({ type: 'start', id: 4 }); transport.start()
    await vi.advanceTimersByTimeAsync(30)
    expect(invoke).toHaveBeenCalledWith('remote_voice_reject', { id: 4 })
    vi.restoreAllMocks()
  })
})
