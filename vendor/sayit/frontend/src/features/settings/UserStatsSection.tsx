import { useEffect, useMemo, useState } from 'react'
import { Card, CardContent } from '@/components/ui/card'
import { summarizeDomainScenes } from '@/services/personalization/userStats'
import type { UserStats } from '@/services/personalization/types'
import { listHistory, type HistoryRecord } from '@/services/store'
import { pickVoiceDurationSec } from '@/services/timeModel'
import { getLocale, t, type TranslationKey } from '@/i18n'
import { useT } from '@/i18n/useT'
import { recordedAppDisplayName } from '@/i18n/displayNames'

type TimeRange = 'today' | '7d' | '30d' | 'all'

const RANGE_OPTIONS: { value: TimeRange; labelKey: TranslationKey }[] = [
  { value: 'today', labelKey: 'range.today' },
  { value: '7d', labelKey: 'range.7d' },
  { value: '30d', labelKey: 'range.30d' },
  { value: 'all', labelKey: 'range.all' },
]

function getStartOfDay(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate())
}

function getRangeCutoff(range: TimeRange): number {
  const now = new Date()
  switch (range) {
    case 'today': return getStartOfDay(now).getTime()
    case '7d': return getStartOfDay(new Date(now.getTime() - 6 * 86400000)).getTime()
    case '30d': return getStartOfDay(new Date(now.getTime() - 29 * 86400000)).getTime()
    case 'all': return 0
  }
}

function formatPercent(value: number) {
  return `${(value * 100).toFixed(1)}%`
}

function formatDate(timestamp: number | undefined) {
  if (!timestamp) return t('stats.unknown')
  return new Date(timestamp).toLocaleDateString(getLocale(), {
    year: 'numeric', month: 'long', day: 'numeric',
  })
}

interface RangeStats {
  totalWords: number
  totalSessions: number
  avgWords: number
  appUsageCount: Record<string, number>
}

function computeRangeStats(records: HistoryRecord[]): RangeStats {
  const appUsageCount: Record<string, number> = {}
  let totalWords = 0
  let totalSessions = 0
  for (const r of records) {
    if (r.isEmpty) continue
    totalSessions++
    totalWords += r.charCount || 0
    if (r.appName || r.appId) {
      const fallback = r.appName || r.appId || t('stats.unknown')
      const key = recordedAppDisplayName(r.appId, fallback)
      appUsageCount[key] = (appUsageCount[key] || 0) + 1
    }
  }
  return {
    totalWords,
    totalSessions,
    avgWords: totalSessions > 0 ? Math.round(totalWords / totalSessions) : 0,
    appUsageCount,
  }
}

export default function UserStatsSection({ userStats }: { userStats: UserStats }) {
  useT()
  const [range, setRange] = useState<TimeRange>('today')
  const [records, setRecords] = useState<HistoryRecord[]>([])

  useEffect(() => {
    listHistory({ limit: 10000 }).then(setRecords)
  }, [])

  const filteredRecords = useMemo(() => {
    if (range === 'all') return records
    const cutoff = getRangeCutoff(range)
    return records.filter((r) => r.timestamp >= cutoff)
  }, [range, records])

  const rangeStats = useMemo(() => computeRangeStats(filteredRecords), [filteredRecords])

  // 全部模式用原始 userStats（包含 domainWords 等），其他范围用聚合数据
  const displayStats = range === 'all' ? {
    totalWords: userStats.totalWords,
    totalSessions: userStats.totalSessions,
    avgWords: userStats.totalSessions > 0 ? Math.round(userStats.totalWords / userStats.totalSessions) : 0,
    appUsageCount: userStats.appUsageCount,
  } : rangeStats

  const topScenes = range === 'all' ? summarizeDomainScenes(userStats, 3) : []
  const appUsageEntries = Object.entries(displayStats.appUsageCount)
    .sort(([, a], [, b]) => b - a)
    .slice(0, 5)

  return (
    <Card>
      <CardContent className="space-y-4 p-6">
        <div className="flex items-start justify-between">
          <div>
            <h2 className="text-lg font-semibold">{t('stats.title')}</h2>
            <p className="mt-1 text-xs text-muted-foreground">
              {t('stats.desc')}
            </p>
          </div>
          <div className="flex gap-0.5">
            {RANGE_OPTIONS.map((opt) => (
              <button
                key={opt.value}
                onClick={() => setRange(opt.value)}
                className={`rounded-md px-2.5 py-1 text-xs transition-colors ${range === opt.value
                  ? 'bg-foreground text-background font-medium'
                  : 'text-muted-foreground hover:bg-accent'
                  }`}
              >
                {t(opt.labelKey)}
              </button>
            ))}
          </div>
        </div>

        <div className="grid gap-3 sm:grid-cols-3">
          <div className="rounded-xl border bg-card px-4 py-3">
            <p className="text-xs text-muted-foreground">{t('stats.outputChars')}</p>
            <p className="mt-1 text-lg font-semibold">{displayStats.totalWords.toLocaleString(getLocale())}</p>
          </div>
          <div className="rounded-xl border bg-card px-4 py-3">
            <p className="text-xs text-muted-foreground">{t('stats.records')}</p>
            <p className="mt-1 text-lg font-semibold">{displayStats.totalSessions.toLocaleString(getLocale())}</p>
          </div>
          <div className="rounded-xl border bg-card px-4 py-3">
            <p className="text-xs text-muted-foreground">{t('stats.avgChars')}</p>
            <p className="mt-1 text-lg font-semibold">{displayStats.avgWords.toLocaleString(getLocale())}</p>
          </div>
        </div>

        {range === 'all' && (userStats.firstUsedAt || userStats.lastUsedAt) && (
          <div className="rounded-xl border bg-card px-4 py-3">
            <div className="flex items-center justify-between text-xs">
              <div>
                <span className="text-muted-foreground">{t('stats.firstUsedLabel')}</span>
                <span className="ml-1 font-medium">{formatDate(userStats.firstUsedAt)}</span>
              </div>
              <div>
                <span className="text-muted-foreground">{t('stats.lastUsedLabel')}</span>
                <span className="ml-1 font-medium">{formatDate(userStats.lastUsedAt)}</span>
              </div>
            </div>
          </div>
        )}

        {range === 'all' && topScenes.length > 0 && (
          <div className="space-y-2">
            <p className="text-xs font-medium text-muted-foreground">{t('stats.top3Scenes')}</p>
            {topScenes.map((scene) => (
              <div key={scene.id} className="flex items-center justify-between rounded-xl border bg-card px-4 py-3">
                <div>
                  <p className="text-sm font-medium">{scene.label}</p>
                  <p className="mt-1 text-xs text-muted-foreground">{t('stats.charsValue', { count: scene.words.toLocaleString(getLocale()) })}</p>
                </div>
                <span className="text-sm font-semibold text-foreground">{formatPercent(scene.ratio)}</span>
              </div>
            ))}
          </div>
        )}

        {appUsageEntries.length > 0 && (
          <div className="space-y-2">
            <p className="text-xs font-medium text-muted-foreground">{t('stats.top5Apps')}</p>
            <div className="space-y-2">
              {appUsageEntries.map(([appId, count]) => (
                <div key={appId} className="flex items-center justify-between rounded-xl border bg-card px-4 py-2.5">
                  <p className="text-sm font-medium">{appId}</p>
                  <span className="text-sm font-semibold text-foreground">{t('stats.timesValue', { count: count.toLocaleString(getLocale()) })}</span>
                </div>
              ))}
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  )
}
