import { useSyncExternalStore } from 'react'
import { subscribeActivePreset, getActivePresetSnapshot } from '@/stores/activePreset'

export function useActivePreset() {
  return useSyncExternalStore(subscribeActivePreset, getActivePresetSnapshot)
}
