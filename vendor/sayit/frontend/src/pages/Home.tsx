import { useEffect, useState, useSyncExternalStore } from 'react'
import { Link } from 'react-router-dom'
import { invoke } from '@tauri-apps/api/core'
import { ArrowUpRight, AudioLines, Bluetooth, Clock, FileText, Mic, Wand2 } from 'lucide-react'
import { getSetting, getStats, listHistory, type HistoryRecord, type Stats } from '@/services/store'
import { getModeStatus, refreshModeStatus, subscribeModeStatus } from '@/stores/modeStatus'
import { subscribeAiEnabled, getAiEnabled } from '@/stores/aiEnabled'
import { getLocale } from '@/i18n'
import { useT } from '@/i18n/useT'

export default function Home() {
  useT()
  const en = getLocale() === 'en'
  const [stats, setStats] = useState<Stats>({ totalDurationSec: 0, totalChars: 0 })
  const [records, setRecords] = useState<HistoryRecord[]>([])
  const [remote, setRemote] = useState<{ phase: string; remoteName?: string } | null>(null)
  const [error, setError] = useState('')
  const [micLabel, setMicLabel] = useState('')
  const mode = useSyncExternalStore(subscribeModeStatus, getModeStatus)
  const aiEnabled = useSyncExternalStore(subscribeAiEnabled, getAiEnabled)
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
  // 输入源明示：遥控器不是必选项——未连接时按住 PTT 键即用系统麦克风录音。
  // label 需要麦克风权限才非空，拿不到就退回"默认麦克风"。
  useEffect(() => {
    let disposed = false
    void (async () => {
      const selected = await getSetting('selectedMic', '').catch(() => '')
      let label = ''
      try {
        const devices = await navigator.mediaDevices.enumerateDevices()
        label = devices.find(d => d.kind === 'audioinput' && d.deviceId === selected)?.label ?? ''
      } catch { /* 无权限/枚举失败 */ }
      if (!disposed) setMicLabel(label)
    })()
    return () => { disposed = true }
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
        <Bluetooth className="mt-1 h-5 w-5 shrink-0 text-primary" />
        <div className="min-w-0 flex-1"><p className="text-xs text-muted-foreground">{en ? 'Remote' : '遥控器'}</p><p className="mt-1 text-sm font-semibold">{remote?.remoteName || (en ? 'No remote connected' : '未连接遥控器')}</p><p className="mt-1 flex items-center gap-1.5 text-xs text-muted-foreground"><span className={`h-1.5 w-1.5 rounded-full ${connected ? 'bg-success' : 'bg-muted-foreground'}`} />{connected ? (en ? 'Connected' : '已连接') : (en ? 'Disconnected' : '未连接')}</p></div>
        <ArrowUpRight className="h-4 w-4 text-muted-foreground group-hover:text-primary" />
      </Link>
      <Link to="/settings" className="group flex min-w-0 items-start gap-3 py-2">
        <Mic className="mt-1 h-5 w-5 shrink-0 text-primary" />
        <div className="min-w-0 flex-1">
          <p className="text-xs text-muted-foreground">{en ? 'Microphone input' : '麦克风输入'}</p>
          <p className="mt-1 text-sm font-semibold">
            {connected
              ? (en ? 'Remote has priority; mic as fallback' : '遥控器优先，麦克风兜底')
              : (micLabel || (en ? 'System default microphone' : '系统默认麦克风'))}
          </p>
          <p className="mt-1 text-xs text-muted-foreground">
            {connected
              ? (en ? 'Remote voice key records from the remote; PTT key still uses this mic' : '遥控器语音键走遥控器录音；按住 PTT 键仍用此麦克风')
              : (en ? 'Hold the PTT key to dictate with this microphone — no remote required' : '无需遥控器：按住 PTT 键即可用此麦克风口述')}
          </p>
        </div>
        <ArrowUpRight className="h-4 w-4 text-muted-foreground group-hover:text-primary" />
      </Link>
      <Link to="/ai-instructions" className="group flex min-w-0 items-start gap-3 py-2">
        <Wand2 className="mt-1 h-5 w-5 shrink-0 text-primary" />
        <div className="min-w-0 flex-1">
          <p className="text-xs text-muted-foreground">{en ? 'AI cleanup' : 'AI 整理'}</p>
          <p className="mt-1 text-sm font-semibold">
            {aiEnabled ? (en ? 'On — transcripts get polished' : '已开启 · 转写后自动整理') : (en ? 'Off — raw transcripts' : '已关闭 · 原文直出')}
          </p>
          <p className="mt-1 flex items-center gap-1.5 text-xs text-muted-foreground">
            <span className={`h-1.5 w-1.5 rounded-full ${aiEnabled ? 'bg-success' : 'bg-muted-foreground'}`} />
            {en ? 'Toggle anytime from the title bar' : '可随时在标题栏开关'}
          </p>
        </div>
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
        records.map(record => (
          <Link key={record.id} to="/history" className="group flex items-start gap-3 border-b py-2.5 hover:bg-muted/40">
            <div className="min-w-0 flex-1">
              <p className="line-clamp-1 text-sm leading-6">{record.llmText || record.asrText || record.failReason}</p>
              <p className="mt-0.5 flex flex-wrap items-center gap-2 text-xs text-muted-foreground"><Clock size={12} />{new Date(record.timestamp).toLocaleString(getLocale())}<span>{(record.durationSec ?? 0).toFixed(1)} s</span></p>
            </div>
            <ArrowUpRight size={14} className="mt-2 shrink-0 text-muted-foreground group-hover:text-primary" />
          </Link>
        ))}
    </section>
  </div>
}
