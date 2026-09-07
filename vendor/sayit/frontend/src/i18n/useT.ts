/**
 * 组件里取翻译函数。
 *
 * 订阅 locale 而不是把整棵树塞进 Context：切语言时靠 useSyncExternalStore 触发
 * 重渲染，任何组件（含悬浮窗那个独立 React 根）都能单独用，不依赖 Provider 位置。
 */
import { useSyncExternalStore } from 'react'
import { getLocale, subscribeLocale, t, type Locale } from '.'

/** 当前界面语言，变化时组件自动重渲染。 */
export function useLocale(): Locale {
  return useSyncExternalStore(subscribeLocale, getLocale, getLocale)
}

/** 返回 `t`。函数身份稳定，重渲染由 locale 变化驱动。 */
export function useT(): typeof t {
  useLocale()
  return t
}
