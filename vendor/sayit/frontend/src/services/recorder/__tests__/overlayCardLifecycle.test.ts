import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({
  presentOverlay: vi.fn((_data: unknown) => Promise.resolve(1)),
  updateOverlay: vi.fn((_data: unknown) => Promise.resolve()),
  hideOverlay: vi.fn(() => Promise.resolve()),
  setEscapeActionMode: vi.fn((_mode: string, _token?: number) => Promise.resolve()),
  setCardHotkeys: vi.fn((_actions: string[], _token?: number) => Promise.resolve()),
}))

vi.mock('../../bridge', () => ({
  presentOverlay: mocks.presentOverlay,
  updateOverlay: mocks.updateOverlay,
  hideOverlay: mocks.hideOverlay,
  setEscapeActionMode: mocks.setEscapeActionMode,
  setCardHotkeys: mocks.setCardHotkeys,
  copyText: vi.fn(() => Promise.resolve()),
}))
vi.mock('../../debugLog', () => ({ addRuntimeEvent: vi.fn() }))
vi.mock('../../store', () => ({
  getSetting: vi.fn((_key: string, fallback: unknown) => Promise.resolve(fallback)),
}))
vi.mock('@/i18n', () => ({
  getLocale: () => 'zh-CN',
  t: (key: string) => key,
}))

// 与 overlayListeningPayload.test.ts 同样的理由：不要用顶层 await import()。
import { OverlayService } from '../OverlayService'

/** Rust 侧 CARD_HOTKEY_TTL_MS = 20s，前端续期间隔 8s。 */
const KEEPALIVE_MS = 8000
const NATIVE_TTL_MS = 20000

function newService() {
  return new OverlayService(() => 0)
}

function hotkeyCalls(): Array<[string[], number]> {
  return mocks.setCardHotkeys.mock.calls.map(([actions, token]) => [actions ?? [], token ?? 0])
}

function escapeModes(): string[] {
  return mocks.setEscapeActionMode.mock.calls.map(([mode]) => String(mode))
}

/** 取最后一次调用的 payload。刻意不用 `.at(-1)` —— 本仓库的 lib 目标是 ES2020。 */
function lastPayload(fn: { mock: { calls: unknown[][] } }): Record<string, unknown> | undefined {
  const calls = fn.mock.calls
  if (calls.length === 0) return undefined
  return calls[calls.length - 1][0] as Record<string, unknown>
}

describe('悬浮窗卡片的生命周期', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    mocks.presentOverlay.mockClear()
    mocks.updateOverlay.mockClear()
    mocks.hideOverlay.mockClear()
    mocks.setEscapeActionMode.mockClear()
    mocks.setCardHotkeys.mockClear()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  /**
   * 这条钉的是 issue #71 的核心改动，**没有任何时长上限**。
   *
   * 旧行为是 10 秒自动隐藏，等于把"文本还没交付"当成"通知已读"：用户去够鼠标的
   * 时间里卡片就没了，而那段文字此刻既不在输入框、也不在剪贴板（不可编辑那条路
   * 刻意不提前改写剪贴板），关了历史的用户更是连别处都找不回来。
   *
   * 所以断言取半小时 —— 任何"到时自动隐藏"的实现都过不了这条。中间一次都不许 hide。
   */
  it('结果卡永不自动消失', () => {
    const service = newService()
    service.showFallback('今天天气怎么样', 'not_editable', 7)

    vi.advanceTimersByTime(30 * 60 * 1000)
    expect(mocks.hideOverlay).not.toHaveBeenCalled()
  })

  /**
   * 失败卡没有要交付的内容，看过就没用了，所以它**可以**自动收起。
   *
   * 5 秒而不是原先的 60 秒：录音已经在历史记录里、原因也在日志里，错过这张卡不丢东西。
   * 60 秒实测的结果是用户手动去关它 —— 那等于把一条通知做成了待办事项。
   */
  it('失败卡 5 秒后自动收起', () => {
    const service = newService()
    service.showFailure({ title: 'recorder.emptyAfterProcessingTitle', recovery: 'none', token: 8 })

    vi.advanceTimersByTime(4_000)
    expect(mocks.hideOverlay).not.toHaveBeenCalled()
    vi.advanceTimersByTime(2_000)
    expect(mocks.hideOverlay).toHaveBeenCalled()
  })

  /**
   * 「识别没出文字」一律走 toast，**不许建卡片**。
   *
   * 断言钉的是机制不是结果：只断言"没有 hideOverlay"或"最终隐藏了"在旧实现上也会绿
   * （旧实现是卡片，60 秒后照样隐藏）。这里要求 presentOverlay 收到的 state 是 toast、
   * 并且整个过程没有注册过任何全局按键接管 —— 卡片一定会注册 Esc。
   */
  it('识别没出文字时只弹 toast，不建卡片、不接管按键', () => {
    for (const reason of ['silent', 'no_text'] as const) {
      mocks.presentOverlay.mockClear()
      mocks.setEscapeActionMode.mockClear()
      const service = newService()

      service.showNoSpeech(reason, { runId: 1 })

      const states = mocks.presentOverlay.mock.calls
        .map((call) => (call[0] as { state?: string } | undefined)?.state)
      expect(states).toEqual(['toast'])
      expect(states).not.toContain('failure')
      // 'off' 是解除接管，不算接管；出现 dismiss_fallback 就说明建了卡片。
      const modes = mocks.setEscapeActionMode.mock.calls.map((call) => call[0])
      expect(modes.every((mode) => mode === 'off')).toBe(true)
    }
  })

  /**
   * 卡片既然不自动消失，正常生命周期就只能靠续期证明"我还在"。
   *
   * 断言的是**机制**而不是结果：Rust 侧那个 TTL 是异常兜底，只断言"按键还能用"
   * 在没有续期的实现上也会全绿（TTL 内本来就能用）。这里要求在超过 TTL 的时刻
   * 之后仍有新的注册调用发出去。
   */
  it('卡片可见期间会持续续期，续期间隔小于原生 TTL', () => {
    const service = newService()
    service.showFallback('今天天气怎么样', 'not_editable', 7)

    const initialHotkeys = hotkeyCalls().length
    const initialEscapes = escapeModes().length
    expect(KEEPALIVE_MS).toBeLessThan(NATIVE_TTL_MS)

    vi.advanceTimersByTime(NATIVE_TTL_MS + KEEPALIVE_MS)

    const renewals = hotkeyCalls().length - initialHotkeys
    expect(renewals).toBeGreaterThanOrEqual(Math.floor(NATIVE_TTL_MS / KEEPALIVE_MS))
    expect(escapeModes().length).toBeGreaterThan(initialEscapes)
    // 续期必须沿用同一张卡片的 token，否则原生侧会把它当成新卡片、重新抓前台窗口。
    for (const [actions, token] of hotkeyCalls()) {
      expect(actions).toEqual(['copy'])
      expect(token).toBe(7)
    }
  })

  it('结果卡注册 Ctrl+C 与 Esc；失败卡只要 Esc', () => {
    const service = newService()
    service.showFallback('今天天气怎么样', 'not_editable', 7)
    expect(hotkeyCalls()[0]).toEqual([['copy'], 7])
    expect(escapeModes()).toContain('dismiss_fallback')

    mocks.setCardHotkeys.mockClear()
    mocks.setEscapeActionMode.mockClear()
    // 失败卡没有文本可交付，注册 Ctrl+C 只会白抢用户的复制键。
    service.showFailure({ title: 'recorder.recognitionFailedTitle', recovery: 'unknown', token: 8 })
    expect(hotkeyCalls().every(([actions]) => actions.length === 0)).toBe(true)
    expect(escapeModes()).toContain('dismiss_fallback')
  })

  it('没有文本的结果卡不注册 Ctrl+C', () => {
    const service = newService()
    service.showFallback('', 'not_editable', 7)
    expect(hotkeyCalls().every(([actions]) => actions.length === 0)).toBe(true)
  })

  /** PTT Lab 的预览卡片没有 Orchestrator 代次，绝不能开启无人消费的全局按键接管。 */
  it('token=0 的预览卡片不接管任何按键', () => {
    const service = newService()
    service.showFallback('预览文本', 'not_editable', 0)

    expect(hotkeyCalls().every(([actions]) => actions.length === 0)).toBe(true)
    expect(escapeModes().every((mode) => mode === 'off')).toBe(true)

    // 也不该留下一个续期定时器。
    mocks.setCardHotkeys.mockClear()
    vi.advanceTimersByTime(NATIVE_TTL_MS * 2)
    expect(mocks.setCardHotkeys).not.toHaveBeenCalled()
  })

  /**
   * 验收项「复制过程中开始新录音」的机制侧：新录音会把悬浮窗切到 waiting，
   * 那一刻卡片的按键接管必须当场解除。
   *
   * 不解除的症状很隐蔽：续期定时器每 8 秒把 Esc 重新设成 dismiss_fallback，
   * 把新录音刚设好的 cancel_recording 覆盖掉 —— 用户在录音中按 Esc，取消的是
   * 一张已经不存在的卡片。
   */
  it('开始新一轮显示会解除上一张卡片的按键接管', () => {
    const service = newService()
    service.showFallback('今天天气怎么样', 'not_editable', 7)

    mocks.setCardHotkeys.mockClear()
    mocks.setEscapeActionMode.mockClear()
    service.showWaiting()

    expect(hotkeyCalls()).toContainEqual([[], 0])

    // 关键：续期定时器必须已经停掉，不能再覆盖新状态的 Esc 模式。
    mocks.setEscapeActionMode.mockClear()
    service.startListeningTicker(9)
    mocks.setEscapeActionMode.mockClear()
    vi.advanceTimersByTime(KEEPALIVE_MS * 3)
    expect(escapeModes()).not.toContain('dismiss_fallback')

    service.stopListeningTicker()
  })

  it('hide 会解除按键接管', () => {
    const service = newService()
    service.showFallback('今天天气怎么样', 'not_editable', 7)

    mocks.setCardHotkeys.mockClear()
    service.hide()

    expect(hotkeyCalls()).toContainEqual([[], 0])
    expect(escapeModes()).toContain('off')
  })

  /**
   * 回归钉：悬浮窗自己收起卡片之后，主窗必须停掉续期。
   *
   * 悬浮窗只是渲染端，卡片的生命周期归这边管。它 hideOverlay() 之后如果不回报，
   * 8 秒后续期定时器又把 Esc / Ctrl+C 注册回来 —— 卡片早就没了，用户的按键却还
   * 被接管着，而且原生侧会重新抓一次**当前**前台窗口（可能已经是别的程序）。
   *
   * 断言方式是"此后不再出现任何非空的注册"，而不是"解除被调用过一次" ——
   * 后者在有 bug 的实现上也会全绿（它确实调过一次，只是随后又注册回来了）。
   */
  it('悬浮窗收起卡片后不再续期', () => {
    const service = newService()
    service.showFallback('今天天气怎么样', 'not_editable', 7)

    service.noteCardDismissed(7)
    mocks.setCardHotkeys.mockClear()
    mocks.setEscapeActionMode.mockClear()

    vi.advanceTimersByTime(KEEPALIVE_MS * 5)
    expect(hotkeyCalls().every(([actions]) => actions.length === 0)).toBe(true)
    expect(escapeModes().every((mode) => mode === 'off')).toBe(true)
  })

  /** 上一张卡片的关闭回报迟到时，不能把现在这张的按键接管一起撤掉。 */
  it('关闭回报只认当前卡片', () => {
    const service = newService()
    service.showFallback('第一段', 'not_editable', 7)
    service.showFallback('第二段', 'not_editable', 9)

    mocks.setCardHotkeys.mockClear()
    service.noteCardDismissed(7)

    expect(mocks.setCardHotkeys).not.toHaveBeenCalled()
    // 第二张卡片照常续期。
    vi.advanceTimersByTime(KEEPALIVE_MS + 100)
    expect(hotkeyCalls()).toContainEqual([['copy'], 9])
  })

  /**
   * 失败卡上那句"可在历史记录里重新识别"必须由实际存档结果决定，所以分两步下发：
   * 先 present（此刻还不知道存没存下来），存档有结论后再 update。
   */
  it('恢复结论是随后补发的，且只认当前卡片的 token', () => {
    const service = newService()
    service.showFailure({ title: 'recorder.protocolIncompleteTitle', recovery: 'unknown', token: 8 })
    expect(lastPayload(mocks.presentOverlay)).toMatchObject({
      state: 'failure',
      failureRecovery: 'unknown',
    })

    service.updateFailureRecovery('history', 8)
    expect(lastPayload(mocks.updateOverlay)).toMatchObject({
      state: 'failure',
      failureRecovery: 'history',
    })

    // 上一张卡片的存档结论迟到时不能改写现在这张。
    mocks.updateOverlay.mockClear()
    service.updateFailureRecovery('history', 7)
    expect(mocks.updateOverlay).not.toHaveBeenCalled()
  })

  /**
   * 补发恢复结论时必须**重新带上标题和原因**。
   *
   * 原生侧的 update 是整份替换 `latest_overlay_payload`，不是合并。只发
   * `{ state, failureRecovery }` 的话，那份"最近状态"底稿会变成一份没有标题的残缺
   * payload，而界面此刻完全正常（渲染端是合并语义）—— 残缺只在有人重放底稿时暴露：
   * overlay 重挂后 `overlay_ready` 会重放它。实测 2026-09-23 的现场就是这个：
   * 卡片显示成「出错了」+「录音已保存」，标题落到兜底文案，原因整行消失。
   *
   * 所以这条断言钉的是**下发的字段**，不是界面效果：只断言"界面正常"在出 bug 的版本上
   * 也是绿的。
   */
  it('补发恢复结论时重新带上标题与原因，不留下残缺的最近状态', () => {
    const service = newService()
    service.showFailure({
      title: 'recorder.recognitionFailedTitle',
      detail: 'err.provider.insufficientBalance',
      recovery: 'unknown',
      token: 8,
    })
    service.updateFailureRecovery('history', 8)

    expect(lastPayload(mocks.updateOverlay)).toMatchObject({
      state: 'failure',
      failureTitle: 'recorder.recognitionFailedTitle',
      failureDetail: 'err.provider.insufficientBalance',
      failureRecovery: 'history',
    })
  })

  it('卡片收起后不再把上一张的标题带进新的下发', () => {
    const service = newService()
    service.showFailure({ title: 'recorder.recognitionFailedTitle', detail: '余额不足', recovery: 'none', token: 8 })
    service.hide()

    // 新一张卡自带标题；关键是 hide 之后旧标题不许残留。
    mocks.presentOverlay.mockClear()
    service.showFailure({ title: 'recorder.processingTimeoutTitle', recovery: 'none', token: 9 })
    expect(lastPayload(mocks.presentOverlay)).toMatchObject({
      failureTitle: 'recorder.processingTimeoutTitle',
      failureDetail: '',
    })
  })

  /** 宽限期提示是胶囊形态的 thinking 变体，Esc 的语义是"放弃等待"而不是"关卡片"。 */
  it('宽限期等待用独立的 Esc 语义', () => {
    const service = newService()
    service.showAwaitingLateResult(15, 11)

    expect(lastPayload(mocks.presentOverlay)).toMatchObject({
      state: 'thinking',
      thinkingNote: 'late',
    })
    expect(escapeModes()).toContain('abandon_late_result')
    // 这一步没有卡片按钮，不该接管 Ctrl+C。
    expect(hotkeyCalls().every(([actions]) => actions.length === 0)).toBe(true)
  })
})
