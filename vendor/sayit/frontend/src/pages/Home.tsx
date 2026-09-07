import { useEffect, useState, useSyncExternalStore } from 'react'
import { Link } from 'react-router-dom'
import { invoke } from '@tauri-apps/api/core'
import { ArrowUpRight, AudioLines, Bluetooth, Clock, FileText } from 'lucide-react'
import { getStats, listHistory, type HistoryRecord, type Stats } from '@/services/store'
import { getModeStatus, refreshModeStatus, subscribeModeStatus } from '@/stores/modeStatus'
import { getLocale } from '@/i18n'
import { useT } from '@/i18n/useT'

export default function Home() {
  useT()
  const en = getLocale() === 'en'
  const [stats, setStats] = useState<Stats>({ totalDurationSec: 0, totalChars: 0 })
  const [records, setRecords] = useState<HistoryRecord[]>([])
  const [remote, setRemote] = useState<{ phase: string; remoteName?: string } | null>(null)
  const [error, setError] = useState('')
  const mode = useSyncExternalStore(subscribeModeStatus, getModeStatus)
  useEffect(() => {
    let disposed = false
    const refresh = async () => {
      try {
        const [s, h, b] = await Promise.all([getStats(), listHistory({ limit: 5 }), invoke<{ phase: string; remoteName?: string }>('get_ble_snapshot')])
        if (!disposed) { setStats(s); setRecords(h); setRemote(b); setError('') }
      } catch (e) { if (!disposed) setError(String(e)) }
    }
    void refresh(); void refreshModeStatus()
    const timer = setInterval(() => void refresh(), 3000)
    return () => { disposed = true; clearInterval(timer) }
  }, [])
  const connected = remote?.phase === 'ready'
  return <div className="mx-auto max-w-4xl">
    <header className="mb-7 flex items-center justify-between border-b pb-5">
      <h1>{en ? 'Workspace' : '工作区'}</h1>
      <span className="text-xs text-muted-foreground">{new Date().toLocaleDateString(getLocale(), { month: 'long', day: 'numeric', weekday: 'short' })}</span>
    </header>
    <div className="grid gap-5 border-b pb-6 sm:grid-cols-2">
      <Link to="/voice-engine" className="group flex min-w-0 items-start gap-3 py-2">
        <AudioLines className="mt-1 h-5 w-5 shrink-0 text-primary" />
        <div className="min-w-0 flex-1"><p className="text-xs text-muted-foreground">{en ? 'Speech engine' : '语音引擎'}</p><p className="mt-1 break-words text-sm font-semibold">{mode.detail || (en ? 'No model selected' : '尚未选择模型')}</p><p className="mt-1 text-xs text-muted-foreground">{mode.ready === false ? mode.blockedReason : mode.mode === 'local' ? (en ? 'On-device recognition' : '本机识别') : mode.mode === 'cloud_api' ? (en ? 'Custom service' : '自定义服务') : (en ? 'Server' : '服务器')}</p></div>
        <ArrowUpRight className="h-4 w-4 text-muted-foreground group-hover:text-primary" />
      </Link>
      <Link to="/remote/connection" className="group flex min-w-0 items-start gap-3 py-2">
        <Bluetooth className="mt-1 h-5 w-5 shrink-0 text-info-strong" />
        <div className="min-w-0 flex-1"><p className="text-xs text-muted-foreground">{en ? 'Remote' : '遥控器'}</p><p className="mt-1 text-sm font-semibold">{remote?.remoteName || (en ? 'No remote connected' : '未连接遥控器')}</p><p className="mt-1 flex items-center gap-1.5 text-xs text-muted-foreground"><span className={`h-1.5 w-1.5 rounded-full ${connected ? 'bg-success' : 'bg-muted-foreground'}`} />{connected ? (en ? 'Connected' : '已连接') : (en ? 'Disconnected' : '未连接')}</p></div>
        <ArrowUpRight className="h-4 w-4 text-muted-foreground group-hover:text-primary" />
      </Link>
    </div>
    <dl className="grid grid-cols-3 divide-x border-b py-6">
      {[{ name: en ? 'Dictation' : '累计口述', value: (stats.totalDurationSec / 60).toFixed(1), unit: en ? 'min' : '分钟' },
        { name: en ? 'Characters' : '累计文字', value: stats.totalChars.toLocaleString(), unit: en ? 'chars' : '字' },
        { name: en ? 'Average speed' : '平均语速', value: stats.totalDurationSec > 60 ? Math.round(stats.totalChars / stats.totalDurationSec * 60) : 0, unit: en ? 'chars/min' : '字/分' }].map((item, index) =>
        <div key={item.name} className={index ? 'pl-5' : ''}><dt className="text-xs text-muted-foreground">{item.name}</dt><dd className="mt-2 text-2xl font-semibold tabular-nums">{item.value}<span className="ml-2 text-xs font-normal text-muted-foreground">{item.unit}</span></dd></div>)}
    </dl>
    <section className="pt-6">
      <div className="mb-3 flex items-center justify-between"><h2 className="font-semibold">{en ? 'Recent transcripts' : '最近转写'}</h2><Link to="/history" className="flex items-center gap-1 text-xs text-muted-foreground hover:text-primary">{en ? 'All history' : '全部记录'}<ArrowUpRight size={14} /></Link></div>
      {error && <p role="alert" className="py-3 text-sm text-destructive-strong">{error}</p>}
      {records.length === 0 ? <div className="flex min-h-40 flex-col items-center justify-center gap-3 text-muted-foreground"><FileText className="h-7 w-7" /><p className="text-sm">{en ? 'No transcripts yet' : '暂无转写记录'}</p><Link to="/voice-engine" className="text-xs text-primary underline">{en ? 'Configure speech engine' : '配置语音引擎'}</Link></div> :
        records.map(record => <Link key={record.id} to="/history" className="block border-b py-4 hover:bg-muted/40"><p className="line-clamp-2 text-sm leading-7">{record.llmText || record.asrText || record.failReason}</p><p className="mt-2 flex flex-wrap items-center gap-2 text-xs text-muted-foreground"><Clock size={12} />{new Date(record.timestamp).toLocaleString(getLocale())}<span>{(record.durationSec ?? 0).toFixed(1)} s</span></p></Link>)}
    </section>
  </div>
}
