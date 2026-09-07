// AI 整理开关的全局状态 store
// 让标题栏与 AI 整理设置页共享同一状态，任一处切换都会同步
// 采用 useSyncExternalStore 模式（与 connectionStatus 一致，无外部依赖）

import { getSetting, setSetting } from '@/services/store'
import { setAiEnabledCache, showAiEnabledToast } from '@/services/recorder'
import * as bridge from '@/services/bridge'

type Listener = () => void

let currentValue = false
let initialized = false
let ready = false
const listeners = new Set<Listener>()

function emitChange() {
  for (const listener of listeners) listener()
}

/** 从持久化设置读取初始值（仅首次生效），供应用启动时调用 */
export async function initAiEnabled(): Promise<void> {
  if (initialized) return
  initialized = true
  const stored = await getSetting('aiEnabled', false)
  const next = Boolean(stored)
  if (next !== currentValue) {
    currentValue = next
    emitChange()
  }
  // 托盘建菜单时自己读了一次库，正常情况下已经对了；这里再对齐一次是兜底
  // （两边的默认值口径若哪天又走偏，至少启动后就能自愈）。
  void bridge.setTrayAiEnabled(next)

  // 托盘右键的「AI 整理」由 Rust 落库后广播过来。监听放在 store 而不是某个组件里：
  // 这条状态的所有写入路径都收在这一个文件，才不会漏掉录音器缓存那一步。
  // 不做取消订阅 —— store 与 webview 同生命周期。
  //
  // 顺序有讲究：**必须在读回初值之后**才开始监听。托盘菜单在前端加载前就能点，
  // 若先监听，一次落在「已读到旧值、还没赋值」窗口里的广播会被随后的旧值覆盖掉；
  // 放在后面则这次点击已经进库，上面那次读取拿到的就是新值。
  bridge.onAiCleanupChanged((enabled) => {
    applyExternalAiEnabled(enabled)
  })
  bridge.onAiCleanupToggleRequested(() => {
    void setAiEnabled(!currentValue, { showToast: true })
  })
  // 初始值已落定：下一帧再置 ready，让标题栏等订阅方在值稳定后才显示/放动画，
  // 避免冷启动时开关从默认(关)跳到已保存(开)被看见。
  const flipReady = () => { ready = true; emitChange() }
  if (typeof requestAnimationFrame === 'function') requestAnimationFrame(flipReady)
  else flipReady()
}

export function getAiEnabled(): boolean {
  return currentValue
}

/** AI 开关初始值是否已从设置读回（供 UI 决定首帧是否显示/放动画） */
export function getAiEnabledReady(): boolean {
  return ready
}

export function subscribeAiEnabled(listener: Listener): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

/** 设置开关状态：更新内存状态 + 通知订阅者 + 持久化 + 同步录音器缓存（轻量，无 IPC 全量刷新）。
 *  持久化写入放到后台执行，不阻塞 UI 反馈，避免快速切换时卡顿。 */
export async function setAiEnabled(next: boolean, options: { showToast?: boolean } = {}): Promise<void> {
  if (next !== currentValue) {
    currentValue = next
    emitChange()
  }
  setAiEnabledCache(next)
  void setSetting('aiEnabled', next)
  // 托盘菜单建好后不再重建，状态只能这样推过去。
  void bridge.setTrayAiEnabled(next)
  if (options.showToast) showAiEnabledToast(next)
}

/** 切换开关状态 */
export async function toggleAiEnabled(): Promise<void> {
  await setAiEnabled(!currentValue)
}

/** 状态已在别处（目前只有托盘右键）改完并落库：只同步内存、订阅方与录音器缓存。
 *  刻意不再写库、也不回写托盘 —— 那两件事 Rust 侧已经做了，重复做会出现两个写者。 */
function applyExternalAiEnabled(next: boolean): void {
  if (next !== currentValue) {
    currentValue = next
    emitChange()
  }
  setAiEnabledCache(next)
}
