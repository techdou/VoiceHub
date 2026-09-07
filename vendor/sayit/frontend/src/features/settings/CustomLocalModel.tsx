import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { FolderOpen, Loader2 } from 'lucide-react'
import { getSetting } from '@/services/store'
import { reconnectProvider } from '@/services/recorder'
import { refreshModeStatus } from '@/stores/modeStatus'

export default function CustomLocalModel() {
  const [path, setPath] = useState('')
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState('')
  useEffect(() => { invoke<string | null>('custom_model_path').then(p => setPath(p ?? '')).catch(e => setMessage(String(e))) }, [])
  async function select() {
    const chosen = await open({ multiple: false, filters: [{ name: 'GGUF ASR', extensions: ['gguf'] }] })
    if (typeof chosen === 'string') setPath(chosen)
  }
  async function save() {
    setBusy(true); setMessage('')
    try {
      const selected = await invoke<string>('register_custom_model', { modelPath: path, accelerator: await getSetting('localAsr.accelerator', 'auto') })
      setPath(selected); setMessage('模型已加载并启用')
      reconnectProvider(); await refreshModeStatus()
      window.dispatchEvent(new Event('sayit:models-dir-changed'))
    } catch (error) { setMessage(String(error)) }
    finally { setBusy(false) }
  }
  return <section className="border-b pb-5">
    <h2 className="mb-3 text-base font-semibold">自定义本地模型</h2>
    <label htmlFor="custom-model-path" className="mb-1 block text-sm text-muted-foreground">GGUF 模型文件</label>
    <div className="flex flex-wrap gap-2">
      <input id="custom-model-path" className="min-w-0 flex-1 basis-48 rounded-md border bg-background px-3 py-2 text-sm" value={path} onChange={e => setPath(e.target.value)} disabled={busy} />
      <button title="选择模型文件" aria-label="选择模型文件" className="rounded-md border p-2" disabled={busy} onClick={() => void select().catch(e => setMessage(String(e)))}><FolderOpen size={18} /></button>
      <button className="flex items-center gap-2 rounded-md bg-primary px-3 text-sm text-primary-foreground" disabled={busy || !path.trim()} onClick={() => void save()}>{busy && <Loader2 size={16} className="animate-spin" />}加载并启用</button>
    </div>
    {message && <p role="status" className="mt-2 break-words text-sm">{message}</p>}
  </section>
}
