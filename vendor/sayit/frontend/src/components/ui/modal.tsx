// 弹窗外壳：Esc 关闭、焦点陷阱、关闭后还原焦点、role="dialog"
//
// 为什么要有这个：设置页里有三个手搓的弹窗（离线下载指引 / 确认删除模型 / 更改模型目录），
// 每个都是 `fixed inset-0 + <div onClick>` 背板，没有 role、没有 aria-modal、按 Esc 没反应、
// 打开后焦点还留在背后的页面上、Tab 能跑到弹窗外面去。确认删除框更极端：唯一的关闭方式
// 是 Tab 到「取消」（背板点击要用鼠标）。语义只在这里修一次。

import { useCallback, useEffect, useRef, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { X } from 'lucide-react'
import { useT } from '@/i18n/useT'
import { cn } from '@/lib/utils'

const FOCUSABLE = 'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'

interface ModalProps {
  /** 弹窗标题，同时作为无障碍名称 */
  title: string
  onClose: () => void
  /** 关闭中（如迁移文件时）禁掉 Esc 与背板点击，避免中断危险操作 */
  locked?: boolean
  /** 是否显示右上角关闭按钮。确认类弹窗用底部按钮，不需要 */
  showCloseButton?: boolean
  /** 面板宽度类，如 'w-[520px]'。会自动补 max-w-[calc(100vw-2rem)] 防小窗溢出 */
  panelClassName?: string
  children: ReactNode
}

export function Modal({
  title,
  onClose,
  locked,
  showCloseButton,
  panelClassName,
  children,
}: ModalProps) {
  const t = useT()
  const panelRef = useRef<HTMLDivElement>(null)
  const titleId = useRef(`modal-title-${Math.random().toString(36).slice(2, 8)}`).current

  const requestClose = useCallback(() => {
    if (!locked) onClose()
  }, [locked, onClose])

  /**
   * 打开时把焦点移进弹窗（否则读屏与键盘用户还停在背后的页面上），关闭时还回去。
   *
   * 必须是空依赖、只跑一次：这段原来和下面的按键监听写在同一个 effect 里，依赖 requestClose，
   * 而 requestClose 跟着调用方传进来的 onClose 变。调用方的 onClose 只要不是 useCallback
   * 包过的（正常写法就不是），每次重渲染都是新函数 → effect 重跑 → 焦点被抢回第一个控件。
   * 表现就是"在输入框里每打一个字，光标就跳回上面的下拉框"。
   *
   * 想指定落焦点的控件，在它（或它的容器）上加 data-modal-autofocus。
   */
  useEffect(() => {
    const previouslyFocused = document.activeElement as HTMLElement | null
    const panel = panelRef.current
    const preferred = panel?.querySelector<HTMLElement>('[data-modal-autofocus]')
    const target = preferred?.matches(FOCUSABLE)
      ? preferred
      : preferred?.querySelector<HTMLElement>(FOCUSABLE)
      ?? panel?.querySelector<HTMLElement>(FOCUSABLE)
      ; (target ?? panel)?.focus()

    return () => { previouslyFocused?.focus?.() }
  }, [])

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.stopPropagation()
        requestClose()
        return
      }
      if (event.key !== 'Tab') return

      // 焦点陷阱：Tab 到头尾时在弹窗内绕回，不跑到背后的页面上
      const focusables = Array.from(panelRef.current?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? [])
      if (focusables.length === 0) return
      const firstEl = focusables[0]
      const lastEl = focusables[focusables.length - 1]
      const active = document.activeElement
      if (event.shiftKey && (active === firstEl || active === panelRef.current)) {
        event.preventDefault()
        lastEl.focus()
      } else if (!event.shiftKey && active === lastEl) {
        event.preventDefault()
        firstEl.focus()
      }
    }

    document.addEventListener('keydown', onKeyDown, true)
    return () => document.removeEventListener('keydown', onKeyDown, true)
  }, [requestClose])

  return createPortal(
    <div
      // 遮罩用固定的黑色而非 --foreground：深色主题的 foreground 是接近白的浅灰，
      // 拿它当遮罩会把背景照亮。遮罩在任何主题下都必须是暗的。
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4"
      onClick={requestClose}
    >
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        onClick={(e) => e.stopPropagation()}
        className={cn(
          'relative max-h-[calc(100vh-2rem)] max-w-[calc(100vw-2rem)] overflow-y-auto rounded-xl border bg-card p-6 shadow-xl',
          panelClassName,
        )}
      >
        {showCloseButton && (
          <button
            type="button"
            onClick={requestClose}
            aria-label={t('window.close')}
            className="absolute right-3 top-3 rounded-md p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            <X className="h-4 w-4" aria-hidden />
          </button>
        )}
        <h3 id={titleId} className="text-base font-semibold">{title}</h3>
        {children}
      </div>
    </div>,
    document.body,
  )
}
