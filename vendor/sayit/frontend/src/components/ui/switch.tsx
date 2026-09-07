import { cn } from '@/lib/utils'

interface SwitchProps {
  checked: boolean
  onChange: () => void
  /** 开关右侧的可见文字，同时作为无障碍名称 */
  label?: string
  /** 没有可见文字时，指向承担名称的元素 id（如卡片标题）。
   *  两者都不给的话，读屏只会念出一个没有名字的「切换按钮」。 */
  labelledBy?: string
  disabled?: boolean
  className?: string
  /** 'sm' 适合紧凑列表行 */
  size?: 'default' | 'sm'
  /** 初始化阶段传 true：跳过颜色/位移过渡，避免「默认值→已保存值」被当成用户操作动画出来 */
  noAnimation?: boolean
  /** 初始化阶段传 true：占位但不可见（用内联 style，不依赖 Tailwind 产出 .invisible 类） */
  hidden?: boolean
}

/**
 * 统一 Switch 开关组件
 * default: h-5 w-9 / sm: h-4 w-7
 */
export function Switch({ checked, onChange, label, labelledBy, disabled, className, size = 'default', noAnimation, hidden }: SwitchProps) {
  const sm = size === 'sm'
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-labelledby={label ? undefined : labelledBy}
      onClick={onChange}
      disabled={disabled}
      style={hidden ? { visibility: 'hidden' } : undefined}
      className={cn('inline-flex items-center gap-2 disabled:cursor-not-allowed disabled:opacity-50', className)}
    >
      <span
        className={cn(
          'relative shrink-0 rounded-full',
          !noAnimation && 'transition-colors',
          sm ? 'h-4 w-7' : 'h-5 w-9',
          checked ? 'bg-primary' : 'bg-muted',
        )}
      >
        <span
          className={cn(
            'absolute left-0.5 top-0.5 rounded-full bg-card shadow',
            !noAnimation && 'transition-transform',
            sm ? 'h-3 w-3' : 'h-4 w-4',
            checked && (sm ? 'translate-x-3' : 'translate-x-4'),
          )}
        />
      </span>
      {label && <span className="text-sm">{label}</span>}
    </button>
  )
}
