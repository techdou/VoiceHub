// 「语音引擎」页里是否有尚未保存的配置改动（useSyncExternalStore 模式）。
//
// 为什么需要跨组件共享：服务地址（ServerSection）和 API Key（CloudAPISection）都只活在
// 各自组件的 local state 里，而「识别测试」（AsrTestSection）是它们的兄弟组件、读的是
// **已保存**的值。用户粘完 key 直接点「开始测试」必然失败，然后误以为 key 是坏的。
// 这个标记让测试入口能在有未保存改动时把自己禁掉并说明原因。
//
// 谁改谁负责清：ServerSection / CloudAPISection 在输入变化与保存成功后各调一次，
// 卸载时（切走路由）复位，避免把"脏"状态留给下一次进入。

type Listener = () => void

let dirty = false
const listeners = new Set<Listener>()

export function getEngineDraftDirty(): boolean {
  return dirty
}

export function subscribeEngineDraft(listener: Listener): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function setEngineDraftDirty(next: boolean): void {
  if (dirty === next) return
  dirty = next
  for (const listener of listeners) listener()
}
