// 列表拖拽排序 —— 用原生 HTML5 拖拽，不引入拖拽库。
//
// 为什么不装 dnd-kit 之类：桌面端只有鼠标（不需要触摸/指针兼容），列表都是简单的一维
// 纵向列表，原生 API 足够；装库要多背几十 KB，而且此前全应用没有任何拖拽依赖。
//
// ⚠️ Tauri 前置条件：窗口必须配 `"dragDropEnabled": false`（tauri.conf.json）。
// Tauri 默认开启自己的「文件拖放」处理器，它在 Windows 上会拦掉 OS 层的拖放，
// 导致 webview 里的 HTML5 拖拽事件根本不触发 —— 表现就是"手柄拖不动"。
// 参考 tauri-apps/tauri#14373。本项目没有用到文件拖放，关掉无副作用。
//
// 两个刻意的设计：
//  1. **只有拖拽柄可拖**，不是整行可拖 —— 行内有输入框（如热词分类的添加框），整行可拖
//     会把框里的文字选择也变成拖拽。拖起来时用 setDragImage 把整行作为拖影，观感不受影响。
//  2. **拖拽柄可聚焦，聚焦后按 ↑/↓ 也能移动** —— 原生拖拽对键盘和读屏用户完全不可用，
//     这样不必在界面上常驻一对上下按钮，也不会把这两类用户挡在外面。

import { useRef, useState, type ComponentPropsWithoutRef, type DragEvent, type KeyboardEvent } from 'react'
import { GripVertical } from 'lucide-react'
import { useT } from '@/i18n/useT'
import { cn } from '@/lib/utils'

export interface SortableOptions {
  /** 把第 from 项移到第 to 项的位置（越界由调用方忽略） */
  onMove: (from: number, to: number) => void
}

export function useSortable({ onMove }: SortableOptions) {
  const t = useT()
  const [dragIndex, setDragIndex] = useState<number | null>(null)
  const [overIndex, setOverIndex] = useState<number | null>(null)
  const rowRefs = useRef<Array<HTMLElement | null>>([])

  const registerRow = (index: number) => (element: HTMLElement | null) => {
    rowRefs.current[index] = element
  }

  /** 加在拖拽柄上 */
  const handleProps = (index: number, label: string) => ({
    draggable: true,
    'aria-label': t('ui.sortableAria', { label }),
    onDragStart: (event: DragEvent) => {
      setDragIndex(index)
      event.dataTransfer.effectAllowed = 'move'
      // 必须写点数据，否则部分平台不认为这是一次有效拖拽
      event.dataTransfer.setData('text/plain', String(index))
      const row = rowRefs.current[index]
      if (row) event.dataTransfer.setDragImage(row, 16, row.offsetHeight / 2)
    },
    onDragEnd: () => {
      setDragIndex(null)
      setOverIndex(null)
    },
    onKeyDown: (event: KeyboardEvent) => {
      if (event.key === 'ArrowUp') {
        event.preventDefault()
        onMove(index, index - 1)
      } else if (event.key === 'ArrowDown') {
        event.preventDefault()
        onMove(index, index + 1)
      }
    },
  })

  /** 加在每一行（拖拽的落点）上 */
  const rowProps = (index: number) => ({
    ref: registerRow(index),
    onDragOver: (event: DragEvent) => {
      if (dragIndex === null) return
      event.preventDefault()
      event.dataTransfer.dropEffect = 'move'
      setOverIndex(index)
    },
    onDragLeave: () => {
      setOverIndex((current) => (current === index ? null : current))
    },
    onDrop: (event: DragEvent) => {
      event.preventDefault()
      if (dragIndex !== null && dragIndex !== index) onMove(dragIndex, index)
      setDragIndex(null)
      setOverIndex(null)
    },
  })

  /** 行的状态样式：拖起来的那行变淡，当前落点给一条指示线 */
  const rowClassName = (index: number) => cn(
    dragIndex === index && 'opacity-40',
    overIndex === index && dragIndex !== null && dragIndex !== index && (
      index > dragIndex ? 'border-b-2 border-b-primary' : 'border-t-2 border-t-primary'
    ),
  )

  return { handleProps, rowProps, rowClassName, dragIndex }
}

/**
 * 拖拽柄。默认淡显，鼠标移到所在行（group）时显现。
 *
 * 用 div + role="button" 而不是真的 <button>：Chromium 对表单控件的拖拽支持不一致，
 * `<button draggable>` 经常不触发 dragstart（另一个"拖不动"的来源）。
 * 配 tabIndex 与 role 后，键盘与读屏的语义不受影响。
 */
export function DragHandle({
  className,
  ...rest
}: ComponentPropsWithoutRef<'div'>) {
  return (
    <div
      role="button"
      tabIndex={0}
      className={cn(
        'shrink-0 cursor-grab select-none rounded p-1 text-muted-foreground/0 transition-colors',
        'group-hover:text-muted-foreground/60 hover:bg-accent hover:text-foreground',
        'focus-visible:text-muted-foreground active:cursor-grabbing',
        className,
      )}
      {...rest}
    >
      <GripVertical className="h-3.5 w-3.5" aria-hidden />
    </div>
  )
}

/** 数组换位：把 from 移到 to（越界原样返回）。拖拽是"插入到某个位置"，不是两两交换。 */
export function moveItem<T>(list: T[], from: number, to: number): T[] {
  if (from === to || from < 0 || to < 0 || from >= list.length || to >= list.length) return list
  const next = [...list]
  const [item] = next.splice(from, 1)
  next.splice(to, 0, item)
  return next
}
