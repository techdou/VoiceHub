// 「点背板关闭弹窗」的判据。
//
// 为什么不能直接在背板上挂 onClick：浏览器派发 click 的目标是 **mousedown 目标与
// mouseup 目标的最近公共祖先**。在弹窗里的输入框按下鼠标、拖动选字时移出面板再松开，
// 两者的公共祖先就是那个 `fixed inset-0` 铺满全屏的背板本身 —— click 直接落在背板上，
// 面板的 stopPropagation 根本不在这条冒泡路径里，压不住，于是弹窗被关掉、草稿全丢。
// 用户反馈的原话是「拖动全选时把鼠标移到小窗口外，松开后窗口就自己关了」。
//
// 判据改成「**按下**那一刻就落在背板本身，并且 click 也落在背板本身」，两个条件都要。
//
// 用 `target === currentTarget` 而不是 `!panel.contains(target)`，是为了对 portal 天然
// 安全：portal 出去的浮层其 DOM target 不是背板，照样判成「不在背板上」，不会误关。

import { useRef, type MouseEvent } from 'react'

export type BackdropPointerEvent =
  | { type: 'mousedown'; onBackdrop: boolean }
  | { type: 'click'; onBackdrop: boolean }

/**
 * 判据本体，抽成纯函数是为了能测 —— 这个仓库没有 jsdom / testing-library，
 * 挂在 DOM 上的那一版没法跑。`armed` 是「上一次按下落在背板本身」。
 *
 * 四条要同时成立（都有对应单测）：
 *  · 点背板空白处（按下与 click 都在背板）→ 关闭；
 *  · 面板内按下、拖出面板松开 → **不关**（本次修的就是这条）；
 *  · 面板内的键盘 click（target 不是背板）→ 不关；
 *  · 背板上按下但拖出窗口外松开（没有 click）之后，面板内的 click 也不能关 ——
 *    所以 click 一到就撤防，无论这次关不关。
 */
export function reduceBackdropDismiss(
  armed: boolean,
  event: BackdropPointerEvent,
): { armed: boolean; dismiss: boolean } {
  if (event.type === 'mousedown') {
    return { armed: event.onBackdrop, dismiss: false }
  }
  if (!armed) return { armed, dismiss: false }
  return { armed: false, dismiss: event.onBackdrop }
}

/**
 * 返回一对要挂在**背板元素**上的事件处理器。
 *
 * ⚠️ 必须挂在背板（那个 `fixed inset-0` 的 div）上，不是面板上 ——
 * `currentTarget` 要等于背板，判据才成立。
 */
export function useBackdropDismiss(onDismiss: () => void) {
  const armedRef = useRef(false)

  const step = (event: BackdropPointerEvent) => {
    const next = reduceBackdropDismiss(armedRef.current, event)
    armedRef.current = next.armed
    if (next.dismiss) onDismiss()
  }

  return {
    onMouseDown: (event: MouseEvent<HTMLElement>) => {
      step({ type: 'mousedown', onBackdrop: event.target === event.currentTarget })
    },
    onClick: (event: MouseEvent<HTMLElement>) => {
      step({ type: 'click', onBackdrop: event.target === event.currentTarget })
    },
  }
}
