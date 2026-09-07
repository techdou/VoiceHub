// 密钥输入框（带「显示/隐藏」切换）
//
// 为什么抽出来：设置页里有两处各自实现了一遍，两处的眼睛按钮都是 tabIndex={-1} 且没有
// 可访问名称——纯键盘用户无法确认自己粘贴的密钥对不对，等于配不了云 API / AI 供应商。
// 收成一个组件后，这条语义只需修一次。

import { useState } from 'react'
import { Eye, EyeOff } from 'lucide-react'
import { useT } from '@/i18n/useT'

interface PasswordInputProps {
  /** 必填：与外部 <label htmlFor> 配对，读屏才能念出这个框叫什么 */
  id: string
  /** 用于组装眼睛按钮的名称，如 'API Key' → 「显示 API Key」 */
  label: string
  value: string
  onChange: (value: string) => void
  /** 回车提交。省略则回车无行为 */
  onSubmit?: () => void
  placeholder?: string
  className?: string
}

export function PasswordInput({
  id,
  label,
  value,
  onChange,
  onSubmit,
  placeholder,
  className,
}: PasswordInputProps) {
  const t = useT()
  const [visible, setVisible] = useState(false)

  return (
    <div className="relative">
      <input
        id={id}
        type={visible ? 'text' : 'password'}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => { if (e.key === 'Enter' && onSubmit) onSubmit() }}
        placeholder={placeholder}
        className={className}
      />
      <button
        type="button"
        onClick={() => setVisible(!visible)}
        aria-label={t(visible ? 'ui.hidePassword' : 'ui.showPassword', { label })}
        aria-pressed={visible}
        className="absolute right-2 top-1/2 -translate-y-1/2 rounded p-0.5 text-muted-foreground transition-colors hover:text-foreground"
      >
        {visible ? <EyeOff className="h-3.5 w-3.5" aria-hidden /> : <Eye className="h-3.5 w-3.5" aria-hidden />}
      </button>
    </div>
  )
}
