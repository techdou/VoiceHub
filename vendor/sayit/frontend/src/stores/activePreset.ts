// 当前润色模式（active preset）的全局响应式状态。
// 让 AI 整理设置页高亮、标题栏状态等多处共享同一状态：
// 无论是页面点击切换，还是全局快捷键切换，都能实时同步刷新。
// 采用 useSyncExternalStore 模式（与 aiEnabled 一致，无外部依赖）。
//
// 注意：此 store 只依赖 services/store，不依赖 services/recorder，避免循环引用。
// 录音器缓存的刷新由调用方（页面/orchestrator）各自负责。

import { getActivePresetId, getPromptPresets } from '@/services/store'

type Listener = () => void

export interface ActivePresetState {
  id: string
  name: string
}

let current: ActivePresetState = { id: 'intent', name: '' }
let initialized = false
const listeners = new Set<Listener>()

function emitChange() {
  for (const listener of listeners) listener()
}

async function resolve(id: string): Promise<ActivePresetState> {
  try {
    const presets = await getPromptPresets()
    const found = presets.find((p) => p.id === id)
    return { id, name: found?.name || '' }
  } catch {
    return { id, name: '' }
  }
}

/** 应用启动时调用，读取初始激活预设 */
export async function initActivePreset(): Promise<void> {
  if (initialized) return
  initialized = true
  const id = await getActivePresetId()
  current = await resolve(id)
  emitChange()
}

export function getActivePresetSnapshot(): ActivePresetState {
  return current
}

export function subscribeActivePreset(listener: Listener): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

/** 用已知的 id/name 立即更新（无 IPC，供快捷键切换即时刷新 UI） */
export function setActivePresetKnown(id: string, name: string): void {
  if (id !== current.id || name !== current.name) {
    current = { id, name }
    emitChange()
  }
}

/** 重新从持久化状态读取当前预设（切换、改名后调用），有变化才通知订阅者 */
export async function refreshActivePreset(): Promise<void> {
  const id = await getActivePresetId()
  const next = await resolve(id)
  if (next.id !== current.id || next.name !== current.name) {
    current = next
    emitChange()
  }
}
