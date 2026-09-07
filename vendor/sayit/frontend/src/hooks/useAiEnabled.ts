import { useSyncExternalStore } from 'react'
import { subscribeAiEnabled, getAiEnabled, getAiEnabledReady } from '@/stores/aiEnabled'

export function useAiEnabled() {
  return useSyncExternalStore(subscribeAiEnabled, getAiEnabled)
}

/** AI 开关初始值是否已就绪（首帧前为 false，用于抑制启动时的开关闪动） */
export function useAiEnabledReady() {
  return useSyncExternalStore(subscribeAiEnabled, getAiEnabledReady)
}
