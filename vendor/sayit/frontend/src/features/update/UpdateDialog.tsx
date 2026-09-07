/**
 * 安装中的遮罩。
 *
 * 只在 installing 那一态出现 —— 此刻应用即将退出、被安装程序覆盖，用户必须知道
 * "别动它"，遮住界面是合理的。
 *
 * 下载期间**故意什么都不显示**：旧实现在下载时也挂这个全屏遮罩，且没有关闭按钮，
 * 于是开机自启后正在按住说话的用户会被突然糊住、接着应用自己退出。现在后台下载
 * 全程无声，下完了才让侧栏的「关于」图标变绿闪烁。
 */

import { useEffect, useState } from 'react'
import { RefreshCw } from 'lucide-react'
import { onAutoUpdateChange, getAutoUpdateState, type AutoUpdateState } from './autoUpdate'
import { useT } from '@/i18n/useT'

export default function UpdateDialog() {
  const t = useT()
  const [state, setState] = useState<AutoUpdateState>(getAutoUpdateState)

  useEffect(() => {
    return onAutoUpdateChange(setState)
  }, [])

  if (state.phase !== 'installing') return null

  const version = state.pending?.version || ''

  return (
    <div className="fixed inset-0 z-[200] flex items-center justify-center bg-background/80 backdrop-blur-sm">
      <div className="w-full max-w-sm rounded-2xl border border-border bg-card p-6 shadow-xl">
        <div className="flex flex-col items-center text-center">
          {/* 用主题的 success 令牌而不是写死 emerald-500：和左下角那枚更新徽标同一个绿，
              而且深浅主题下各自有合适的明度（写死的 emerald 在暗色主题上偏闷）。 */}
          <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-full bg-success/10">
            <RefreshCw className="h-7 w-7 animate-spin text-success" />
          </div>
          <h3 className="text-lg font-semibold">{t('update.installingTitle', { version })}</h3>
          <p className="mt-2 text-sm text-muted-foreground">{t('update.installingDesc')}</p>
          <p className="mt-3 text-xs text-muted-foreground/60">{t('update.doNotClose')}</p>
        </div>
      </div>
    </div>
  )
}
