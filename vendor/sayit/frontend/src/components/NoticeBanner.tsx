// 远程公告 banner — 展示服务端下发的通知，与更新机制解耦。
// 文案纯文本渲染，链接仅 https 且用系统浏览器打开（详见 services/notice.ts 的安全约束）。

import { useEffect, useState } from 'react'
import { Info, AlertTriangle, Megaphone, X } from 'lucide-react'
import { open as shellOpen } from '@tauri-apps/plugin-shell'
import { t } from '@/i18n'
import { useLocale } from '@/i18n/useT'
import {
  fetchActiveNotice,
  dismissNotice,
  isSafeHttpsUrl,
  type NoticeLevel,
  type RemoteNotice,
} from '@/services/notice'

// 每 3 小时刷新一次，长期开着也能收到新公告
const REFRESH_MS = 3 * 60 * 60 * 1000

const LEVEL_STYLES: Record<NoticeLevel, { wrap: string; accent: string; Icon: typeof Info }> = {
  info: {
    wrap: 'border-blue-500/40 bg-blue-500/10',
    accent: 'text-blue-500',
    Icon: Info,
  },
  warning: {
    wrap: 'border-amber-500/40 bg-amber-500/10',
    accent: 'text-amber-500',
    Icon: AlertTriangle,
  },
  critical: {
    wrap: 'border-red-500/40 bg-red-500/10',
    accent: 'text-red-500',
    Icon: Megaphone,
  },
}

/** 去掉末尾斜杠，展示更干净的网址 */
function prettyUrl(url: string): string {
  return url.replace(/\/+$/, '')
}

export default function NoticeBanner() {
  const locale = useLocale()
  const [notice, setNotice] = useState<RemoteNotice | null>(null)

  useEffect(() => {
    let alive = true
    const load = () => {
      void fetchActiveNotice(__APP_VERSION__, locale).then((n) => {
        if (alive) setNotice(n)
      })
    }
    load()
    const timer = setInterval(load, REFRESH_MS)
    return () => {
      alive = false
      clearInterval(timer)
    }
  }, [locale])

  if (!notice) return null

  const style = LEVEL_STYLES[notice.level] ?? LEVEL_STYLES.info
  const Icon = style.Icon
  const canDismiss = notice.dismissible !== false
  const hasLink = isSafeHttpsUrl(notice.linkUrl)
  const url = hasLink ? (notice.linkUrl as string) : ''
  const linkText = notice.linkLabel || prettyUrl(url)

  const handleDismiss = () => {
    void dismissNotice(notice.id)
    setNotice(null)
  }

  const handleOpen = () => {
    if (hasLink) void shellOpen(url)
  }

  return (
    <div className={`mb-6 flex items-start gap-3 rounded-xl border px-4 py-3.5 ${style.wrap}`}>
      <Icon className={`mt-0.5 h-5 w-5 shrink-0 ${style.accent}`} />

      <div className="min-w-0 flex-1">
        <p className="text-sm font-semibold text-foreground">{notice.title}</p>

        {(notice.body || hasLink) && (
          <p className="mt-1 whitespace-pre-line text-xs leading-relaxed text-muted-foreground">
            {notice.body}
            {hasLink && (
              <>
                {notice.body ? ' ' : ''}
                {/* 链接自然融入正文，普通超链接样式；点击用系统浏览器打开 */}
                <button
                  type="button"
                  onClick={handleOpen}
                  title={url}
                  className="font-medium text-primary underline underline-offset-2 transition-opacity hover:opacity-80"
                >
                  {linkText}
                </button>
              </>
            )}
          </p>
        )}
      </div>

      {canDismiss && (
        <button
          type="button"
          onClick={handleDismiss}
          className="shrink-0 rounded-full p-1 text-muted-foreground/60 transition-colors hover:bg-foreground/10 hover:text-foreground"
          aria-label={t('notice.dismiss')}
        >
          <X className="h-4 w-4" />
        </button>
      )}
    </div>
  )
}
