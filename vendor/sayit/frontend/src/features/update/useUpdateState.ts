/**
 * 订阅 autoUpdate 的全局单例状态。
 *
 * autoUpdate.ts 的状态是模块级单例 + 手写订阅（不是 React store），所以这里用
 * useSyncExternalStore 桥一下。getAutoUpdateState 返回的对象只在 setState 时换引用，
 * 满足 getSnapshot 的稳定性要求。
 */

import { useSyncExternalStore } from 'react'
import { getAutoUpdateState, onAutoUpdateChange, type AutoUpdateState } from './autoUpdate'

export function useUpdateState(): AutoUpdateState {
  return useSyncExternalStore(onAutoUpdateChange, getAutoUpdateState)
}
