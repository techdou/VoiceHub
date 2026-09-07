// 使用统计页面

import { useEffect, useMemo, useState } from 'react'
import { Card, CardContent } from '@/components/ui/card'
import { getStats, listHistory, type Stats, type HistoryRecord } from '@/services/store'
import { createDefaultUserStats } from '@/services/personalization/defaults'
import { getUserStats } from '@/services/personalization/store'
import type { UserStats } from '@/services/personalization/types'
import { pickVoiceDurationSec } from '@/services/timeModel'
import { getLocale, t, type TranslationKey } from '@/i18n'
import { useT } from '@/i18n/useT'
import { recordedAppDisplayName, recordedPromptPresetDisplayName } from '@/i18n/displayNames'

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

function formatDuration(sec: number) {
  if (sec < 60) return t('stats.durationSeconds', { sec })
  if (sec < 3600) return t('stats.durationMinutes', { min: Math.floor(sec / 60), sec: sec % 60 })
  const h = Math.floor(sec / 3600)
  const m = Math.floor((sec % 3600) / 60)
  return t('stats.durationHours', { hour: h, min: m })
}

function formatDate(ts: number | undefined) {
  if (!ts) return '-'
  return new Date(ts).toLocaleDateString(getLocale(), { year: 'numeric', month: 'long', day: 'numeric' })
}

/** 计算两个时间戳之间的天数 */
function daysBetween(a: number, b: number) {
  return Math.max(1, Math.ceil(Math.abs(b - a) / 86400000))
}

const WORK_MODE_LABEL_KEYS: Record<string, TranslationKey> = {
  server: 'record.modeServer',
  cloud_api: 'record.modeCloudApi',
  local: 'record.modeLocal',
}

interface FullStats {
  totalChars: number
  totalDurationSec: number
  recordCount: number
  avgCharsPerSession: number
  avgSpeed: number
  savedTimeSec: number
  maxDurationSec: number
  dailyAvgRecords: number
  dailyAvgChars: number
  appUsage: Map<string, number>
  hourBuckets: number[]
  workModeCount: Map<string, number>
  presetCount: Map<string, number>
}

function computeFullStats(records: HistoryRecord[], rangeDays: number): FullStats {
  let totalChars = 0
  let totalDurationSec = 0
  let validCount = 0
  let maxDurationSec = 0
  const appUsage = new Map<string, number>()
  const hourBuckets = new Array(24).fill(0)
  const workModeCount = new Map<string, number>()
  const presetCount = new Map<string, number>()
  const daySet = new Set<string>()

  for (const r of records) {
    const d = new Date(r.timestamp)
    daySet.add(`${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`)
    hourBuckets[d.getHours()]++

    if (r.workMode) workModeCount.set(r.workMode, (workModeCount.get(r.workMode) || 0) + 1)
    if (r.promptPresetName) {
      const presetName = recordedPromptPresetDisplayName(r.promptPresetId, r.promptPresetName)
      presetCount.set(presetName, (presetCount.get(presetName) || 0) + 1)
    }

    // 应用统计放在 isEmpty 之前，确保所有记录都计入
    if (r.appName || r.appId) {
      const fallback = r.appName || r.appId || t('stats.unknown')
      const name = recordedAppDisplayName(r.appId, fallback).replace(/\.exe$/i, '')
      appUsage.set(name, (appUsage.get(name) || 0) + 1)
    }

    if (r.isEmpty) continue
    validCount++
    totalChars += r.charCount || 0
    const dur = pickVoiceDurationSec({ holdSec: r.durationSec, audioSec: r.audioDurationSec, asrSec: r.asrDurationSec })
    totalDurationSec += dur
    if (dur > maxDurationSec) maxDurationSec = dur
  }

  const activeDays = Math.max(1, rangeDays > 0 ? Math.min(rangeDays, daySet.size || 1) : daySet.size || 1)

  return {
    totalChars,
    totalDurationSec: Math.round(totalDurationSec),
    recordCount: records.length,
    avgCharsPerSession: validCount > 0 ? Math.round(totalChars / validCount) : 0,
    avgSpeed: totalDurationSec > 60 ? Math.round(totalChars / (totalDurationSec / 60)) : 0,
    savedTimeSec: Math.round(totalChars / 50) * 60,
    maxDurationSec: Math.round(maxDurationSec),
    dailyAvgRecords: Math.round((records.length / activeDays) * 10) / 10,
    dailyAvgChars: Math.round(totalChars / activeDays),
    appUsage,
    hourBuckets,
    workModeCount,
    presetCount,
  }
}

/** 时段分布柱状图 */
function HourChart({ buckets }: { buckets: number[] }) {
  const max = Math.max(...buckets, 1)
  const periods = [
    { label: t('stats.bucket.night'), sum: buckets.slice(0, 6).reduce((a, b) => a + b, 0) },
    { label: t('stats.bucket.morning'), sum: buckets.slice(6, 12).reduce((a, b) => a + b, 0) },
    { label: t('stats.bucket.afternoon'), sum: buckets.slice(12, 18).reduce((a, b) => a + b, 0) },
    { label: t('stats.bucket.evening'), sum: buckets.slice(18, 24).reduce((a, b) => a + b, 0) },
  ]
  const totalSum = periods.reduce((a, p) => a + p.sum, 0) || 1

  return (
    <div className="space-y-3">
      <div className="flex items-end gap-[3px]" style={{ height: 64 }}>
        {buckets.map((count, hour) => (
          <div key={hour} className="group relative flex-1">
            <div
              className="w-full rounded-sm bg-primary/20 transition-colors group-hover:bg-primary/30"
              style={{ height: `${Math.max(count > 0 ? 4 : 1, (count / max) * 60)}px` }}
            />
            <div className="pointer-events-none absolute -top-7 left-1/2 -translate-x-1/2 rounded bg-foreground px-1.5 py-0.5 text-[10px] text-background opacity-0 transition-opacity group-hover:opacity-100 whitespace-nowrap">
              {t('stats.hourTooltip', { hour, count })}
            </div>
          </div>
        ))}
      </div>
      <div className="flex justify-between text-[10px] text-muted-foreground px-0.5">
        <span>0</span><span>6</span><span>12</span><span>18</span><span>24</span>
      </div>
      <div className="grid grid-cols-4 gap-2">
        {periods.map((p) => (
          <div key={p.label} className="rounded-lg bg-muted/30 px-2.5 py-2 text-center">
            <p className="text-[11px] text-muted-foreground">{p.label}</p>
            <p className="text-sm font-medium">{p.sum}</p>
            <p className="text-[10px] text-muted-foreground">{Math.round((p.sum / totalSum) * 100)}%</p>
          </div>
        ))}
      </div>
    </div>
  )
}

/** 分布列表（统一柔和色调） */
function DistributionList({ entries }: { entries: [string, number][] }) {
  const maxCount = entries.length > 0 ? entries[0][1] : 1

  return (
    <div className="space-y-2">
      {entries.map(([label, count]) => (
        <div key={label} className="flex items-center gap-3">
          <span className="w-20 shrink-0 truncate text-sm">{label}</span>
          <div className="relative h-6 flex-1 overflow-hidden rounded-md bg-muted/20">
            <div
              className="absolute inset-y-0 left-0 rounded-md bg-primary/15"
              style={{ width: `${Math.max((count / maxCount) * 100, 3)}%` }}
            />
          </div>
          <span className="w-16 shrink-0 text-right text-xs tabular-nums text-muted-foreground">
            {t('stats.timesValue', { count })}
          </span>
        </div>
      ))}
    </div>
  )
}

export default function PersonalizationPage() {
  useT()
  const [range, setRange] = useState<TimeRange>('all')
  const [allStats, setAllStats] = useState<Stats>({ totalDurationSec: 0, totalChars: 0 })
  const [userStats, setUserStats] = useState<UserStats>(createDefaultUserStats())
  const [records, setRecords] = useState<HistoryRecord[]>([])

  useEffect(() => {
    getStats().then(setAllStats)
    getUserStats().then(setUserStats)
    listHistory({ limit: 10000 }).then(setRecords)
  }, [])

  const filteredRecords = useMemo(() => {
    if (range === 'all') return records
    const cutoff = getRangeCutoff(range)
    return records.filter((r) => r.timestamp >= cutoff)
  }, [range, records])

  const rangeDays = range === 'today' ? 1 : range === '7d' ? 7 : range === '30d' ? 30 : 0
  const stats = useMemo(() => computeFullStats(filteredRecords, rangeDays), [filteredRecords, rangeDays])

  // "全部"模式下合并 userStats 中的应用数据（历史记录可能缺少早期的 appName 字段）
  const appUsageEntries = useMemo(() => {
    const merged = new Map(stats.appUsage)
    if (range === 'all') {
      for (const [appId, count] of Object.entries(userStats.appUsageCount)) {
        const name = appId.replace(/\.exe$/i, '')
        const existing = merged.get(name) || 0
        if (count > existing) merged.set(name, count)
      }
    }
    return [...merged.entries()].sort(([, a], [, b]) => b - a)
  }, [stats.appUsage, range, userStats.appUsageCount])

  const workModeEntries: [string, number][] = [...stats.workModeCount.entries()]
    .map(([k, v]) => [WORK_MODE_LABEL_KEYS[k] ? t(WORK_MODE_LABEL_KEYS[k]) : k, v] as [string, number])
    .sort(([, a], [, b]) => b - a)

  const presetEntries = [...stats.presetCount.entries()].sort(([, a], [, b]) => b - a)

  // 首次使用距今天数
  const usageDays = userStats.firstUsedAt ? daysBetween(userStats.firstUsedAt, Date.now()) : 0

  return (
    <div className="mx-auto max-w-4xl p-8">
      <div className="mb-6">
        <h1 className="text-2xl font-bold">{t('stats.title')}</h1>
        <div className="mt-3 inline-flex gap-1 rounded-lg border border-border p-0.5">
          {RANGE_OPTIONS.map((opt) => (
            <button
              key={opt.value}
              onClick={() => setRange(opt.value)}
              className={`rounded-md px-3 py-1 text-xs transition-all ${range === opt.value
                ? 'bg-accent text-foreground font-medium'
                : 'text-muted-foreground hover:text-foreground'
                }`}
            >
              {t(opt.labelKey)}
            </button>
          ))}
        </div>
      </div>

      <div className="space-y-5">
        {/* 核心数据 4 列 */}
        <Card>
          <CardContent className="p-6">
            {/* 首次使用提示（仅全部模式，融入顶部） */}
            {range === 'all' && userStats.firstUsedAt && (
              <p className="mb-4 text-xs text-muted-foreground">
                {t('stats.sinceHint', { date: formatDate(userStats.firstUsedAt), days: usageDays })}
              </p>
            )}

            <div className="grid grid-cols-2 gap-x-6 gap-y-4 sm:grid-cols-4">
              <div>
                <p className="text-xs text-muted-foreground">{t('stats.totalChars')}</p>
                <p className="mt-0.5 text-xl font-semibold">{stats.totalChars.toLocaleString()}</p>
              </div>
              <div>
                <p className="text-xs text-muted-foreground">{t('stats.historyRecords')}</p>
                <p className="mt-0.5 text-xl font-semibold">{t('stats.recordsValue', { count: stats.recordCount.toLocaleString() })}</p>
              </div>
              <div>
                <p className="text-xs text-muted-foreground">{t('stats.dictationTime')}</p>
                <p className="mt-0.5 text-xl font-semibold">{formatDuration(stats.totalDurationSec)}</p>
              </div>
              <div>
                <p className="text-xs text-muted-foreground">{t('stats.savedTime')}</p>
                <p className="mt-0.5 text-xl font-semibold">{formatDuration(stats.savedTimeSec)}</p>
              </div>
            </div>

            <div className="mt-4 grid grid-cols-2 gap-x-6 gap-y-3 border-t border-border pt-4 sm:grid-cols-4">
              <div>
                <span className="text-xs text-muted-foreground">{t('stats.avgChars')}</span>
                <p className="text-sm font-medium">{stats.avgCharsPerSession}</p>
              </div>
              <div>
                <span className="text-xs text-muted-foreground">{t('stats.avgSpeed')}</span>
                <p className="text-sm font-medium">{t('stats.speedValue', { value: stats.avgSpeed })}</p>
              </div>
              <div>
                <span className="text-xs text-muted-foreground">{t('stats.longestTake')}</span>
                <p className="text-sm font-medium">{stats.maxDurationSec > 0 ? formatDuration(stats.maxDurationSec) : '-'}</p>
              </div>
              <div>
                <span className="text-xs text-muted-foreground">{t('stats.dailyAverage')}</span>
                <p className="text-sm font-medium">{t('stats.dailyAverageValue', { records: stats.dailyAvgRecords, chars: stats.dailyAvgChars })}</p>
              </div>
            </div>
          </CardContent>
        </Card>

        {/* 使用时段分布 */}
        {stats.recordCount > 0 && (
          <Card>
            <CardContent className="p-6">
              <h2 className="mb-4 text-lg font-semibold">{t('stats.timeOfDay')}</h2>
              <HourChart buckets={stats.hourBuckets} />
            </CardContent>
          </Card>
        )}

        {/* 工作模式 & Prompt 预设 并排 */}
        {(workModeEntries.length > 0 || presetEntries.length > 0) && (
          <div className="grid gap-5 sm:grid-cols-2">
            {workModeEntries.length > 0 && (
              <Card>
                <CardContent className="p-6">
                  <h2 className="mb-3 text-lg font-semibold">{t('stats.workMode')}</h2>
                  <DistributionList entries={workModeEntries} />
                </CardContent>
              </Card>
            )}
            {presetEntries.length > 0 && (
              <Card>
                <CardContent className="p-6">
                  <h2 className="mb-3 text-lg font-semibold">{t('stats.promptPreset')}</h2>
                  <DistributionList entries={presetEntries} />
                </CardContent>
              </Card>
            )}
          </div>
        )}

        {/* 应用使用次数 */}
        {appUsageEntries.length > 0 && (
          <Card>
            <CardContent className="p-6">
              <h2 className="mb-3 text-lg font-semibold">{t('stats.appUsage')}</h2>
              <div className="space-y-2">
                {appUsageEntries.map(([appName, count]) => {
                  const maxCount = appUsageEntries[0][1]
                  const total = appUsageEntries.reduce((a, [, c]) => a + c, 0) || 1
                  const pct = Math.max((count / maxCount) * 100, 3)
                  return (
                    <div key={appName} className="flex items-center gap-3">
                      <span className="w-28 shrink-0 truncate text-sm">{appName}</span>
                      <div className="relative h-6 flex-1 overflow-hidden rounded-md bg-muted/20">
                        <div
                          className="absolute inset-y-0 left-0 rounded-md bg-primary/15"
                          style={{ width: `${pct}%` }}
                        />
                      </div>
                      <span className="w-16 shrink-0 text-right text-xs tabular-nums text-muted-foreground">
                        {t('stats.timesValue', { count })}
                      </span>
                    </div>
                  )
                })}
              </div>
            </CardContent>
          </Card>
        )}
      </div>
    </div>
  )
}
