import { describe, it, expect } from 'vitest'
import { reduceBackdropDismiss, type BackdropPointerEvent } from '../useBackdropDismiss'

/** 按顺序喂一串事件，返回每一步是否触发了关闭 */
function play(events: BackdropPointerEvent[]): boolean[] {
  let armed = false
  return events.map((event) => {
    const next = reduceBackdropDismiss(armed, event)
    armed = next.armed
    return next.dismiss
  })
}

const downOn = { type: 'mousedown', onBackdrop: true } as const
const downOff = { type: 'mousedown', onBackdrop: false } as const
const clickOn = { type: 'click', onBackdrop: true } as const
const clickOff = { type: 'click', onBackdrop: false } as const

describe('点背板关闭弹窗的判据', () => {
  it('在背板空白处点一下 → 关闭', () => {
    expect(play([downOn, clickOn])).toEqual([false, true])
  })

  /**
   * 这条就是用户报的 bug：在输入框里按下鼠标拖动选字，中途移出面板才松开。
   * 浏览器把 click 派发到「按下目标与松开目标的最近公共祖先」= 铺满全屏的背板，
   * 所以 click 确实落在背板上（onBackdrop: true），但**按下**不在背板上。
   */
  it('面板内按下、拖到面板外松开 → 不关（草稿不能就这么丢了）', () => {
    expect(play([downOff, clickOn])).toEqual([false, false])
  })

  it('整个交互都在面板内 → 不关', () => {
    expect(play([downOff, clickOff])).toEqual([false, false])
  })

  /**
   * 在背板上按下、拖出应用窗口外松开：不会产生 click，撤防的机会没了。
   * 若不在 click 到达时无条件撤防，这个悬着的标志会被下一次面板内的
   * 键盘 click（Enter 触发的合成 click）捡走，变成一次莫名的关闭。
   */
  it('背板按下但没等到 click，之后面板内的 click 不能关', () => {
    expect(play([downOn, clickOff, clickOff])).toEqual([false, false, false])
  })

  it('关过一次之后要重新按下才能再关', () => {
    expect(play([downOn, clickOn, clickOn])).toEqual([false, true, false])
  })

  it('mousedown 自己永远不关闭 —— 要等松开，否则拖选背景文字也会关', () => {
    expect(play([downOn, downOn])).toEqual([false, false])
  })
})
