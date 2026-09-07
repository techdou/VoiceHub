// 统一的操作结果反馈块（成功 / 失败 / 警告 / 提示）
//
// 为什么要有这个组件：设置页原来把成功和失败塞进同一个 `text-muted-foreground` 的
// <p> 里，靠读文字才能分辨——「已保存，连接成功」和「已保存，但连接失败」长得一模一样。
// 而另一处（AIProviderSection）又自己手搓了红/绿框并硬编码 red-50/green-50，
// 第三个主题拿到的是从没为它设计过的颜色。这里收敛成一个走 token 的组件。
//
// 颜色规则见 src/themes/types.ts：底用 --xxx 的 10% 淡色，文字用 --xxx-strong
// （对比度达标版），描边用 --xxx 的 30%。

import type { ReactNode } from 'react'
import { CheckCircle2, XCircle, AlertTriangle, Info } from 'lucide-react'
import { cn } from '@/lib/utils'

export type FeedbackTone = 'success' | 'error' | 'warning' | 'info'

/**
 * 每个 tone 的完整类名写成字面量：Tailwind 的 content 扫描不认识拼接出来的类名。
 *
 * 配色为什么这么淡：这个应用整体是中性灰白，页面上最重的色块只是一枚状态圆点。
 * 一开始这里用的是 10% 饱和度色铺满整块（info 更是纯天蓝），结果它比页面上任何
 * 东西都响，跟周围格格不入。现在的分工是：
 *   - 底和描边尽量中性，沿用应用里既有的提示块写法（border-border + bg-muted/40）；
 *   - 只有需要被立刻注意到的 warning / error 才带一点点色（5% 底 + 25% 描边）；
 *   - 「是成功还是失败」由**图标的颜色和形状**承担，不靠整块底色喊。
 * 文字统一用 foreground，保证在任何主题下都读得清。
 */
const TONE_STYLES: Record<
  FeedbackTone,
  { box: string; text: string; icon: string; Icon: typeof Info }
> = {
  success: {
    box: 'border-border bg-muted/40',
    text: 'text-foreground',
    icon: 'text-success-strong',
    Icon: CheckCircle2,
  },
  error: {
    box: 'border-destructive/25 bg-destructive/5',
    text: 'text-foreground',
    icon: 'text-destructive-strong',
    Icon: XCircle,
  },
  warning: {
    box: 'border-warning/25 bg-warning/5',
    text: 'text-foreground',
    icon: 'text-warning-strong',
    Icon: AlertTriangle,
  },
  info: {
    box: 'border-border bg-muted/40',
    text: 'text-muted-foreground',
    icon: 'text-muted-foreground',
    Icon: Info,
  },
}

export interface FeedbackAction {
  label: string
  onClick: () => void
  disabled?: boolean
}

interface FeedbackProps {
  tone: FeedbackTone
  /** 主文案：说清发生了什么，以及下一步该往哪查 */
  message: ReactNode
  /** 原始异常等排查信息，次要字号；能看懂的人需要它，看不懂的人可以忽略 */
  detail?: string
  /** 直接可点的下一步，最多两个 */
  actions?: FeedbackAction[]
  className?: string
}

/**
 * 行内格式提示（贴在输入框下面的一行小字）。
 *
 * 原来两处各写了一遍 `text-amber-500`：白底约 2.15:1，是全页最不可读的一行，
 * 而它偏偏是"你可能粘错了"这种最该被看见的话；同时绕过了主题系统，
 * 第三个注册主题拿到的是从没为它设计过的颜色。
 */
export function FormatHint({ text }: { text: string }) {
  return (
    <p className="mt-1.5 flex items-start gap-1 text-xs text-warning-strong">
      <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0" aria-hidden />
      {text}
    </p>
  )
}

export function Feedback({ tone, message, detail, actions, className }: FeedbackProps) {
  const style = TONE_STYLES[tone]
  const Icon = style.Icon

  return (
    <div
      // 失败要让读屏立刻播报；成功/提示用 polite 排队，不打断用户正在做的事
      role={tone === 'error' ? 'alert' : 'status'}
      aria-live={tone === 'error' ? 'assertive' : 'polite'}
      className={cn('rounded-md border px-3 py-2.5', style.box, className)}
    >
      <div className="flex gap-2">
        <Icon className={cn('mt-0.5 h-4 w-4 shrink-0', style.icon)} aria-hidden />
        <div className="min-w-0 flex-1">
          <p className={cn('text-sm leading-relaxed', style.text)}>{message}</p>

          {/* detail 原来是 font-mono：在中文 Windows 上 CJK 会落到宋体，一整块提示里
              突然冒出一段宋体，比等宽带来的好处难看得多（而且它现在承载的多是中文短句，
              不只是原始异常）。改回正文字体 + 保留换行，长 URL 用 break-words 兜住。 */}
          {detail && (
            <p className="mt-1.5 whitespace-pre-line break-words text-xs leading-relaxed text-muted-foreground">
              {detail}
            </p>
          )}

          {actions && actions.length > 0 && (
            <div className="mt-2 flex flex-wrap gap-2">
              {actions.map((action) => (
                <button
                  key={action.label}
                  type="button"
                  onClick={action.onClick}
                  disabled={action.disabled}
                  // 动作按钮走中性描边，和应用里的 outline 按钮一致，不跟着 tone 变色
                  className="rounded-md border border-border bg-card px-2.5 py-1 text-xs font-medium text-foreground transition-colors hover:bg-accent disabled:pointer-events-none disabled:opacity-50"
                >
                  {action.label}
                </button>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
