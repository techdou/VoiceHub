// 远程公告服务 — 与更新机制完全解耦的服务端可控通知通道。
//
// 即使自动更新出问题、或不想让用户更新软件，也能通过后端下发一条公告，
// 在 App 内提示用户（例如引导去官网/GitHub 手动下载新版）。
//
// 安全约束（远程内容视为不可信输入）：
//   - 文案一律按纯文本渲染（React 默认转义），绝不当 HTML 注入。
//   - 链接只允许 https，且用系统浏览器打开，绝不导航 WebView 自身。

import { getBackendBaseUrl } from './runtimeConfig'
import { getSetting, setSetting } from './store'
import { getLocale, type Locale } from '@/i18n'

export type NoticeLevel = 'info' | 'warning' | 'critical'

export interface RemoteNotice {
  /** 唯一 id：用户关闭后按 id 记忆，不再重复弹 */
  id: string
  /** 级别，决定 banner 配色与图标 */
  level: NoticeLevel
  title: string
  body?: string
  /** 行动按钮链接（仅 https） */
  linkUrl?: string
  linkLabel?: string
  /** 只对 >= minVersion 的客户端显示 */
  minVersion?: string | null
  /** 只对 <= maxVersion 的客户端显示（例如只提示旧版用户去手动更新） */
  maxVersion?: string | null
  /** 生效时间窗（ISO 字符串），可选 */
  startAt?: string | null
  endAt?: string | null
  /** 是否允许关闭，默认 true */
  dismissible?: boolean
}

interface RemoteNoticeTranslation {
  title?: string
  body?: string
  linkLabel?: string
}

interface RemoteNoticePayload extends RemoteNotice {
  /** Optional locale overrides. Top-level strings remain the fallback for old clients. */
  translations?: Partial<Record<Locale, RemoteNoticeTranslation>>
}

const DISMISSED_KEY = 'dismissedNoticeIds'

function compareVersions(a: string, b: string): number {
  const pa = a.split('.').map(Number)
  const pb = b.split('.').map(Number)
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] || 0) - (pb[i] || 0)
    if (d !== 0) return d
  }
  return 0
}

function isTranslation(value: unknown): value is RemoteNoticeTranslation {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false
  const item = value as Record<string, unknown>
  return ['title', 'body', 'linkLabel'].every((key) => (
    item[key] === undefined || typeof item[key] === 'string'
  ))
}

function isValidNotice(v: unknown): v is RemoteNoticePayload {
  if (!v || typeof v !== 'object') return false
  const n = v as Record<string, unknown>
  const levelOk = n.level === 'info' || n.level === 'warning' || n.level === 'critical'
  const translations = n.translations as Record<string, unknown> | undefined
  const translationsOk = translations === undefined || (
    translations !== null
    && typeof translations === 'object'
    && !Array.isArray(translations)
    && Object.entries(translations).every(([locale, value]) => (
      (locale === 'zh-CN' || locale === 'en') && isTranslation(value)
    ))
  )
  return typeof n.id === 'string' && n.id.length > 0
    && typeof n.title === 'string' && n.title.length > 0
    && (n.body === undefined || typeof n.body === 'string')
    && (n.linkLabel === undefined || typeof n.linkLabel === 'string')
    && translationsOk
    && levelOk
}

/** Resolve a server payload into display-ready text while keeping legacy string fields valid. */
export function normalizeRemoteNotice(value: unknown, locale: Locale): RemoteNotice | null {
  if (!isValidNotice(value)) return null
  const translation = value.translations?.[locale]
  return {
    ...value,
    title: translation?.title || value.title,
    body: translation?.body ?? value.body,
    linkLabel: translation?.linkLabel ?? value.linkLabel,
  }
}

function matchesVersion(notice: RemoteNotice, current: string): boolean {
  if (notice.minVersion && compareVersions(current, notice.minVersion) < 0) return false
  if (notice.maxVersion && compareVersions(current, notice.maxVersion) > 0) return false
  return true
}

function withinTimeWindow(notice: RemoteNotice, now = Date.now()): boolean {
  if (notice.startAt) {
    const t = Date.parse(notice.startAt)
    if (!Number.isNaN(t) && now < t) return false
  }
  if (notice.endAt) {
    const t = Date.parse(notice.endAt)
    if (!Number.isNaN(t) && now > t) return false
  }
  return true
}

async function loadRaw(): Promise<unknown> {
  // 开发预览：dev 环境下可在控制台执行
  //   localStorage.setItem('__devNotice', JSON.stringify({ id:'t1', level:'warning', title:'测试公告', body:'内容…', linkUrl:'https://sayitapp.site', linkLabel:'前往官网' }))
  // 刷新即可看到 banner 效果，无需后端。清除：localStorage.removeItem('__devNotice')
  if (import.meta.env.DEV) {
    try {
      const dev = localStorage.getItem('__devNotice')
      if (dev) return JSON.parse(dev)
    } catch {
      /* ignore */
    }
  }

  const baseUrl = getBackendBaseUrl()
  const resp = await fetch(`${baseUrl}/api/notice`, {
    cache: 'no-store',
    signal: AbortSignal.timeout(8000),
  })
  if (!resp.ok) return null
  return await resp.json()
}

/** 拉取当前应展示的公告；无公告/不适用当前版本/不在时间窗/已被用户关闭 → 返回 null。 */
export async function fetchActiveNotice(
  currentVersion: string,
  locale: Locale = getLocale(),
): Promise<RemoteNotice | null> {
  try {
    const raw = await loadRaw()
    const notice = normalizeRemoteNotice(raw, locale)
    if (!notice) return null
    if (!matchesVersion(notice, currentVersion)) return null
    if (!withinTimeWindow(notice)) return null
    if (notice.dismissible !== false) {
      const dismissed = await getSetting<string[]>(DISMISSED_KEY, [])
      if (Array.isArray(dismissed) && dismissed.includes(notice.id)) return null
    }
    return notice
  } catch {
    return null
  }
}

/** 记住某条公告已被用户关闭（按 id）。 */
export async function dismissNotice(id: string): Promise<void> {
  const dismissed = await getSetting<string[]>(DISMISSED_KEY, [])
  const list = Array.isArray(dismissed) ? dismissed : []
  if (!list.includes(id)) {
    // 只保留最近 50 条，避免无限增长
    await setSetting(DISMISSED_KEY, [...list, id].slice(-50))
  }
}

/** 校验是否为安全的 https 外链。 */
export function isSafeHttpsUrl(url: string | undefined): url is string {
  if (!url) return false
  try {
    return new URL(url).protocol === 'https:'
  } catch {
    return false
  }
}
