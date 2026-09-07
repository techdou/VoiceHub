// WebDAV 自动备份的设置面板。
//
// 分工：**卡面回答「现在在做什么」，弹窗回答「怎么配」。**
//
// 这张卡片原先把两件事混在一起——一次性配置服务器，和日常查看/触发备份。前者做完
// 就再也不碰，常驻在设置页上是最主要的噪声来源（地址、账号、密码、坚果云说明、测试
// 按钮、测试结果，七组元素占了半张卡，而它们一年用一次）。
//
// 现在：
// - 卡面：一句说明 + 「连到了哪、备份什么、多久一次」一行摘要 + 一个主动作 + 一行状态。
//   没配置时只有说明和右上角一个按钮 —— 绝大多数用户永远不会开这个功能，默认状态
//   不该给他们一整面控件。
// - 弹窗：地址凭证、**备份内容**、频率、保留份数。备份内容必须在这里，因为首次配置
//   时用户就该看见「会上传哪些东西」并当场选择，而不是点了完成之后才在卡面上发现。
//   三项（配置/历史/录音）全部列出，配置那一项显示「始终包含」而不是藏起来——
//   「会同步哪些文件」这个问题要有一个看得见的完整答案。
// - 录音打开时**就地**出现一行提示，不弹二级确认框：弹窗里再套弹窗是反模式，而且
//   常驻提示比点一次就消失的确认更有用（只要录音是开着的，代价就一直写在那儿）。
//
// 功能一个没少，收进弹窗的仍然可达，只是不再和日常操作抢注意力。
//
// 另外两条与视觉无关但同样刻意的：
// - 档位（含不含历史/录音）只存在设置里，手动备份和定时备份都读设置，界面上勾了
//   什么就一定是上传的内容。
// - 录音默认关。录音库能有几个 GB，而网盘有月上传流量额度，默认打开等于替用户做了
//   一个很贵的决定。

import { useEffect, useState } from 'react'
import { CloudUpload, Loader2, RotateCcw, Settings2 } from 'lucide-react'
import { cn } from '@/lib/utils'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Modal } from '@/components/ui/modal'
import { PasswordInput } from '@/components/ui/password-input'
import { Segmented } from '@/components/ui/segmented'
import { Switch } from '@/components/ui/switch'
import { Tooltip } from '@/components/ui/tooltip'
import { FormatHint } from '@/components/ui/feedback'
import { getSetting, setSetting } from '@/services/store'
import { restartApp } from '@/services/backup'
import {
  describeWebDavError,
  listWebDavBackups,
  onWebDavBackupProgress,
  restoreWebDavBackup,
  testWebDavConnection,
  DEFAULT_DAV_URL,
  type WebDavEntry,
  type WebDavLastResult,
  type WebDavProgress,
} from '@/services/webdavBackup'
import {
  onWebDavBackupChange,
  refreshLastResult,
  runBackupNow,
} from '@/features/backup/autoWebdavBackup'
import { formatBytes } from '@/lib/utils'
import type { TranslationKey } from '@/i18n'
import { useT } from '@/i18n/useT'

const inputClass =
  'h-9 w-full rounded-md border border-input-border bg-input-bg px-3 text-sm transition-colors focus:border-input-focus-border'

/**
 * 卡上图标按钮，与「AI 供应商」服务卡上的那几颗同一套样式。
 *
 * 与那边的一个区别：这里**不做 hover 才显形**。那边网格里有三五张卡，图标常显会很吵；
 * 这里只有一张卡，藏起来只会让「立即备份」变得找不到。
 */
const cardIconButtonClass =
  'rounded p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-40'

const phaseLabelKeys: Record<WebDavProgress['phase'], TranslationKey> = {
  preparing: 'backup.status.preparing',
  packingData: 'backup.status.packingData',
  packingAudio: 'backup.status.packingAudio',
  finalizing: 'backup.status.finalizing',
  uploading: 'webdav.status.uploading',
  verifying: 'webdav.status.verifying',
  completed: 'backup.status.completed',
  failed: 'backup.status.failed',
}

const intervalOptions = [
  { value: 24, labelKey: 'webdav.interval.daily' },
  { value: 72, labelKey: 'webdav.interval.every3Days' },
  { value: 168, labelKey: 'webdav.interval.weekly' },
] as const satisfies readonly { value: number; labelKey: TranslationKey }[]

/**
 * 四种档位各有一个完整句子，而不是把「配置」「历史记录」拼起来。
 * 拼接在中文里勉强能读，换到别的语言就会出「Settings and 历史记录」式的畸形。
 */
const contentSummaryKeys: Record<string, TranslationKey> = {
  'false,false': 'webdav.summary.config',
  'true,false': 'webdav.summary.configHistory',
  'false,true': 'webdav.summary.configAudio',
  'true,true': 'webdav.summary.all',
}

type Busy = 'test' | 'list' | 'restore' | null

/** 卡面上只需要认出「连到了哪」。协议头永远是 https，显示它等于八个字符的噪声。 */
function shortServer(url: string): string {
  return url.trim().replace(/^https:\/\//i, '').replace(/\/+$/, '')
}

/** 摘要行里的时间。完整的 toLocaleString 在 11px 一行里太占地方，年份也没人看。 */
function shortTime(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  })
}

export default function WebDavSection() {
  const t = useT()

  // ready 控制「值到位才显示」，animate 控制「隔两帧才允许过渡」。
  // 合成一个标志修不掉自己动一下的问题：揭开的同一帧加回 transition，浏览器会把
  // 「默认值 → 已保存值」真的动画一遍（见 pitfalls 里设置类控件那条）。
  const [ready, setReady] = useState(false)
  const [animate, setAnimate] = useState(false)

  const [enabled, setEnabled] = useState(false)
  const [url, setUrl] = useState('')
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [includeHistory, setIncludeHistory] = useState(false)
  const [includeAudio, setIncludeAudio] = useState(false)
  const [intervalHours, setIntervalHours] = useState(24)
  const [keepCount, setKeepCount] = useState(5)

  const [busy, setBusy] = useState<Busy>(null)
  const [running, setRunning] = useState(false)
  const [progress, setProgress] = useState<WebDavProgress | null>(null)
  const [lastResult, setLastResult] = useState<WebDavLastResult | null>(null)
  const [testMessage, setTestMessage] = useState('')
  const [actionError, setActionError] = useState('')

  const [serverOpen, setServerOpen] = useState(false)
  const [restoreList, setRestoreList] = useState<WebDavEntry[] | null>(null)
  const [restoreTarget, setRestoreTarget] = useState<WebDavEntry | null>(null)
  const [restoreDone, setRestoreDone] = useState(false)

  useEffect(() => {
    let cancelled = false
    void (async () => {
      const [en, u, name, pass, hist, audio, interval, keep] = await Promise.all([
        getSetting('webdav.enabled', false).catch(() => false),
        getSetting('webdav.url', '').catch(() => ''),
        getSetting('webdav.username', '').catch(() => ''),
        getSetting('webdav.password', '').catch(() => ''),
        getSetting('webdav.includeHistory', false).catch(() => false),
        getSetting('webdav.includeAudio', false).catch(() => false),
        getSetting('webdav.intervalHours', 24).catch(() => 24),
        getSetting('webdav.keepCount', 5).catch(() => 5),
      ])
      if (cancelled) return
      setEnabled(en)
      // 地址预填坚果云，但**必须同时落库**：Rust 侧备份时是直接读库的，而 getSetting
      // 的默认值只活在前端内存里。用户只填了账号密码、没碰过地址栏时，库里就没有这
      // 一行，备份会以「地址为空」失败 —— 而界面上明明显示着地址。
      if (u.trim()) {
        setUrl(u)
      } else {
        setUrl(DEFAULT_DAV_URL)
        await setSetting('webdav.url', DEFAULT_DAV_URL).catch(() => undefined)
      }
      setUsername(name)
      setPassword(pass)
      setIncludeHistory(hist)
      setIncludeAudio(audio)
      if (interval === 24 || interval === 72 || interval === 168) setIntervalHours(interval)
      if (keep === 3 || keep === 5 || keep === 10) setKeepCount(keep)
      setLastResult(await refreshLastResult())
      setReady(true)
      // 隔两帧再允许过渡：第一帧用来把已保存值画上去。
      requestAnimationFrame(() => requestAnimationFrame(() => { if (!cancelled) setAnimate(true) }))
    })()
    return () => { cancelled = true }
  }, [])

  useEffect(() => {
    const unlisten = onWebDavBackupProgress(setProgress)
    return () => { void unlisten.then((fn) => fn()) }
  }, [])

  // 后台定时备份也会改状态，面板要跟着动，否则用户看到的是打开设置那一刻的快照。
  useEffect(() => {
    const off = onWebDavBackupChange((state) => {
      setRunning(state.running)
      setLastResult(state.lastResult)
    })
    return off
  }, [])

  // 三样都填了就算配好了。弹窗里边打边存，所以这个值也正好是「测试连接」能不能点。
  const configured = Boolean(url.trim() && username.trim() && password)
  const isBusy = busy !== null || running

  /**
   * 地址栏的就地告警。
   *
   * 说明文字里刻意不写「必须是 https」——填对的人不需要被告知规则，填错的人才需要，
   * 而那时一句针对性的告警比一条常驻的规则有用。所以这里要覆盖**所有**非 https 的
   * 情况，而不只是 `http://`：只查 http 的话，粘一个裸域名（`dav.jianguoyun.com/dav`）
   * 会一路没有提示，直到点测试连接才报错。
   */
  const urlWarning = (() => {
    const value = url.trim().toLowerCase()
    if (!value) return null
    if (value.startsWith('http://')) return t('webdav.error.urlInsecure')
    if (!value.startsWith('https://')) return t('webdav.error.urlScheme')
    return null
  })()

  const patch = async <T,>(key: string, value: T, apply: (value: T) => void) => {
    apply(value)
    await setSetting(key, value)
  }

  const closeServerDialog = () => {
    setServerOpen(false)
    setTestMessage('')
    setActionError('')
  }

  const handleTest = async () => {
    setBusy('test')
    setTestMessage('')
    setActionError('')
    try {
      const count = await testWebDavConnection({ url, username, password })
      setTestMessage(t('webdav.testOk', { count }))
    } catch (error) {
      setActionError(describeWebDavError(error))
    } finally {
      setBusy(null)
    }
  }

  const handleOpenRestore = async () => {
    setBusy('list')
    setActionError('')
    try {
      setRestoreList(await listWebDavBackups())
    } catch (error) {
      setActionError(describeWebDavError(error))
    } finally {
      setBusy(null)
    }
  }

  const handleRestore = async () => {
    if (!restoreTarget) return
    const name = restoreTarget.name
    setRestoreTarget(null)
    setRestoreList(null)
    setBusy('restore')
    setActionError('')
    try {
      await restoreWebDavBackup(name)
      setRestoreDone(true)
      // 与本地导入一致：略作停留让用户看到提示，再重启使更改生效。
      setTimeout(() => { void restartApp() }, 1500)
    } catch (error) {
      setActionError(describeWebDavError(error))
      setBusy(null)
    }
  }

  const uploading = progress?.status === 'running'
  const percent = Math.max(0, Math.min(100, progress?.percent ?? 0))
  const contentSummary = t(contentSummaryKeys[`${includeHistory},${includeAudio}`])
  const intervalLabel = t(
    intervalOptions.find((opt) => opt.value === intervalHours)?.labelKey ?? 'webdav.interval.daily',
  )

  /**
   * 状态徽标，写法沿用「AI 供应商」卡片上的那一枚（同样的圆角、字号、配色语汇）。
   * 备份中只显示百分比 —— 阶段名（「正在打包录音文件…」）太长，塞进徽标会把地址挤掉，
   * 它落在下面的进度行里。
   */
  const chip = uploading
    ? { text: `${Math.round(percent)}%`, box: 'bg-warning/10 text-warning-strong' }
    : lastResult?.ok
      ? { text: t('webdav.state.ok'), box: 'bg-success/10 text-success-strong' }
      : lastResult
        ? { text: t('webdav.state.failed'), box: 'bg-destructive/10 text-destructive-strong' }
        : { text: t('webdav.state.never'), box: 'bg-muted text-muted-foreground' }

  // 摘要：备份什么 · 多久一次（仅自动备份开启时）· 上次什么时候 · 多大
  const metaParts = [contentSummary]
  if (enabled) metaParts.push(intervalLabel)
  if (lastResult) metaParts.push(shortTime(lastResult.at))
  if (lastResult?.ok) metaParts.push(formatBytes(lastResult.bytes))

  return (
    <Card>
      <CardContent className="p-6">
        {/* items-center 是本页卡片头部的既有写法（音频卡、日志卡都是），右侧控件对着
            标题+说明整块居中。用 items-start 会让 32px 高的按钮压着 28px 的标题行，
            视觉上往下坠；而且「配置服务器」按钮和「自动备份」开关高度不同，
            顶对齐时两个状态的右侧内容会错开。 */}
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <div className="min-w-0">
            <h2 className="text-lg font-semibold">{t('webdav.title')}</h2>
            <p className="mt-1 text-sm text-muted-foreground">{t('webdav.desc')}</p>
          </div>
          {/* 右上角这一格是这张卡的开关位：没配置时是唯一的入口，配置完就换成自动备份开关。
              入口按钮单独放在说明下面会显得像一个孤立的动作，和卡片其它内容对不上。 */}
          <div className="flex shrink-0 items-center gap-2">
            {configured ? (
              <>
                <span className="text-sm text-muted-foreground" id="webdav-enabled-label">
                  {t('webdav.autoLabel')}
                </span>
                <Switch
                  checked={enabled}
                  labelledBy="webdav-enabled-label"
                  onChange={() => void patch('webdav.enabled', !enabled, setEnabled)}
                  noAnimation={!animate}
                  hidden={!ready}
                />
              </>
            ) : (
              <Button variant="outline" size="sm" className="gap-1.5" onClick={() => setServerOpen(true)}>
                <Settings2 className="h-4 w-4" />
                {t('webdav.configure')}
              </Button>
            )}
          </div>
        </div>

        {configured && (
          <>
            {/* 备份目标做成一张有边框的小卡，写法沿用「AI 供应商」的服务卡：
                地址 + 状态徽标一行，摘要一行。原先这些是三段没有容器的散排文字，
                夹在标题和按钮之间，读起来像还在写说明，而不是「这是已经配好的那个东西」。
                自动备份开启时用 border-primary + bg-primary/5 —— 这是本仓库表示
                「这一份正在被使用」的既有语汇，不额外增加元素。 */}
            <div
              className={cn(
                'mt-4 rounded-lg border p-3',
                enabled ? 'border-primary bg-primary/5' : 'border-border',
              )}
            >
              <div className="flex items-center gap-2">
                <span className="min-w-0 flex-1 truncate text-xs font-medium" title={url}>
                  {shortServer(url)}
                </span>
                <span
                  className={cn(
                    'inline-flex shrink-0 items-center rounded-md px-1.5 py-0.5 text-[11px] font-medium tabular-nums',
                    chip.box,
                  )}
                >
                  {chip.text}
                </span>
              </div>

              {/* 三个动作收成卡上的图标 + tooltip，和「AI 供应商」服务卡一致。
                  原先是卡片下面三颗并排的文字按钮：一个实心加两个 ghost，宽窄不齐、
                  又和卡片抢层级，而它们本来就是「对这台服务器做的三件事」，属于这张卡。 */}
              <div className="mt-1 flex items-center gap-2">
                <span className="min-w-0 flex-1 truncate text-[11px] text-muted-foreground">
                  {uploading && progress ? t(phaseLabelKeys[progress.phase]) : metaParts.join(' · ')}
                </span>
                <div className="flex shrink-0 items-center gap-0.5">
                  <Tooltip content={running ? t('webdav.backingUp') : t('webdav.backupNow')}>
                    <button
                      type="button"
                      onClick={() => void runBackupNow('manual')}
                      disabled={isBusy}
                      aria-label={running ? t('webdav.backingUp') : t('webdav.backupNow')}
                      className={cardIconButtonClass}
                    >
                      {running
                        ? <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden />
                        : <CloudUpload className="h-3.5 w-3.5" aria-hidden />}
                    </button>
                  </Tooltip>
                  <Tooltip content={t('webdav.restore')}>
                    <button
                      type="button"
                      onClick={() => void handleOpenRestore()}
                      disabled={isBusy}
                      aria-label={t('webdav.restore')}
                      className={cardIconButtonClass}
                    >
                      {busy === 'list' || busy === 'restore'
                        ? <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden />
                        : <RotateCcw className="h-3.5 w-3.5" aria-hidden />}
                    </button>
                  </Tooltip>
                  <Tooltip content={t('webdav.settings')}>
                    <button
                      type="button"
                      onClick={() => setServerOpen(true)}
                      aria-label={t('webdav.settings')}
                      className={cardIconButtonClass}
                    >
                      <Settings2 className="h-3.5 w-3.5" aria-hidden />
                    </button>
                  </Tooltip>
                </div>
              </div>

              {uploading && progress && (
                <div
                  className="mt-1.5 h-1 overflow-hidden rounded-full bg-muted"
                  role="progressbar"
                  aria-label={t('webdav.progressAria')}
                  aria-valuenow={Math.round(percent)}
                  aria-valuemin={0}
                  aria-valuemax={100}
                >
                  <div
                    className="h-full rounded-full bg-primary transition-[width]"
                    style={{ width: `${percent}%` }}
                  />
                </div>
              )}

              {/* 失败原因单独一行：它可能很长，挤进摘要会把前面的信息顶掉。
                  成功和失败都要能看见 —— 少了失败那一半，一个静默失败几个月的
                  备份在界面上和一个正常工作的备份长得一模一样。 */}
              {!uploading && lastResult && !lastResult.ok && (
                <p className="mt-1.5 text-[11px] text-destructive">
                  {describeWebDavError(lastResult.error ?? '')}
                </p>
              )}

              {actionError && <p className="mt-1.5 text-[11px] text-destructive">{actionError}</p>}
            </div>
          </>
        )}
      </CardContent>

      {serverOpen && (
        <Modal
          title={t('webdav.settingsTitle')}
          onClose={closeServerDialog}
          showCloseButton
          panelClassName="w-[520px]"
        >
          <div className="space-y-4">
            <div>
              <label htmlFor="webdav-url" className="mb-1 block text-sm text-muted-foreground">
                {t('webdav.url')}
              </label>
              <input
                id="webdav-url"
                type="url"
                inputMode="url"
                value={url}
                onChange={(e) => void patch('webdav.url', e.target.value, setUrl)}
                placeholder={DEFAULT_DAV_URL}
                className={inputClass}
              />
              {urlWarning
                ? <FormatHint text={urlWarning} />
                : <p className="mt-1 text-[11px] text-muted-foreground">{t('webdav.urlHint')}</p>}
            </div>

            <div className="grid gap-4 sm:grid-cols-2">
              <div>
                <label htmlFor="webdav-username" className="mb-1 block text-sm text-muted-foreground">
                  {t('webdav.username')}
                </label>
                <input
                  id="webdav-username"
                  type="text"
                  autoComplete="off"
                  value={username}
                  onChange={(e) => void patch('webdav.username', e.target.value, setUsername)}
                  className={inputClass}
                />
              </div>
              <div data-modal-autofocus>
                <label htmlFor="webdav-password" className="mb-1 block text-sm text-muted-foreground">
                  {t('webdav.password')}
                </label>
                <PasswordInput
                  id="webdav-password"
                  label={t('webdav.password')}
                  value={password}
                  onChange={(value) => void patch('webdav.password', value, setPassword)}
                  onSubmit={() => void handleTest()}
                  className={inputClass}
                />
              </div>
            </div>

            {/* 坚果云是目前验证过的服务商，它那个必踩的点（要用应用密码）值得占一行。 */}
            <p className="text-[11px] text-muted-foreground">{t('webdav.jianguoyunHint')}</p>
          </div>

          {/* ── 备份内容 ──
              三项全部列出，配置那一项标「始终包含」而不是不显示：「会同步哪些文件」
              这个问题需要一个看得见的完整答案，而不是从两个开关反推。 */}
          <div className="mt-5 border-t border-border pt-4">
            <p className="text-sm font-medium">{t('webdav.contentTitle')}</p>

            <div className="mt-3 flex items-center justify-between gap-3">
              <span className="text-sm">{t('webdav.includeConfig')}</span>
              <span className="shrink-0 text-xs text-muted-foreground">{t('webdav.always')}</span>
            </div>

            <div className="mt-3 flex items-center justify-between gap-3">
              <label className="text-sm" id="webdav-history-label">{t('webdav.includeHistory')}</label>
              <Switch
                checked={includeHistory}
                labelledBy="webdav-history-label"
                onChange={() => void patch('webdav.includeHistory', !includeHistory, setIncludeHistory)}
                noAnimation={!animate}
              />
            </div>

            <div className="mt-3 flex items-center justify-between gap-3">
              <label className="text-sm" id="webdav-audio-label">{t('webdav.includeAudio')}</label>
              <Switch
                checked={includeAudio}
                labelledBy="webdav-audio-label"
                onChange={() => void patch('webdav.includeAudio', !includeAudio, setIncludeAudio)}
                noAnimation={!animate}
              />
            </div>
            {/* 就地提示而不是二级确认框：弹窗里再套弹窗是反模式，而且只要录音是开着的，
                代价就应该一直写在那儿，而不是点一次就消失。 */}
            {includeAudio && <FormatHint text={t('webdav.audioOnHint')} />}
          </div>

          {/* ── 频率与保留 ── */}
          <div className="mt-4 space-y-3 border-t border-border pt-4">
            <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
              <label className="text-sm text-muted-foreground">{t('webdav.intervalLabel')}</label>
              <Segmented
                label={t('webdav.intervalLabel')}
                value={intervalHours}
                options={intervalOptions.map((opt) => ({ value: opt.value, label: t(opt.labelKey) }))}
                onChange={(value) => void patch('webdav.intervalHours', value, setIntervalHours)}
                className="shrink-0 justify-end"
              />
            </div>
            <div>
              <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
                <label className="text-sm text-muted-foreground">{t('webdav.keepLabel')}</label>
                <Segmented
                  label={t('webdav.keepLabel')}
                  value={keepCount}
                  options={[3, 5, 10].map((value) => ({ value, label: String(value) }))}
                  onChange={(value) => void patch('webdav.keepCount', value, setKeepCount)}
                  className="shrink-0 justify-end"
                />
              </div>
              {/* 「保留份数」本身说不清会不会删东西。每次备份是一个新文件、不覆盖旧的，
                  所以必须明说超出的那些会被删掉 —— 用户拿这个数做决定，就得知道代价。 */}
              <p className="mt-1 text-[11px] text-muted-foreground">{t('webdav.keepHint')}</p>
            </div>
          </div>

          <div className="mt-5 flex items-center justify-end gap-2">
            {/* 测试结果和报错共用左边这一格，两者不会同时有意义。 */}
            <span className={`min-w-0 flex-1 text-xs ${actionError ? 'text-destructive' : 'text-success-strong'}`}>
              {actionError || testMessage}
            </span>
            <Button
              variant="outline"
              size="sm"
              className="shrink-0 gap-1.5"
              onClick={() => void handleTest()}
              disabled={busy === 'test' || !configured}
            >
              {busy === 'test' && <Loader2 className="h-4 w-4 animate-spin" />}
              {t('webdav.test')}
            </Button>
            <Button size="sm" className="shrink-0" onClick={closeServerDialog}>
              {t('webdav.done')}
            </Button>
          </div>
        </Modal>
      )}

      {restoreList && (
        <Modal title={t('webdav.restoreTitle')} onClose={() => setRestoreList(null)} panelClassName="w-[460px]">
          {restoreList.length === 0 ? (
            <p className="text-sm text-muted-foreground">{t('webdav.restoreEmpty')}</p>
          ) : (
            <ul className="max-h-64 space-y-1 overflow-y-auto">
              {restoreList.map((entry) => (
                <li key={entry.name}>
                  <button
                    type="button"
                    onClick={() => setRestoreTarget(entry)}
                    className="flex w-full items-center justify-between gap-3 rounded-md px-3 py-2 text-left text-sm transition-colors hover:bg-accent"
                  >
                    <span className="min-w-0 truncate">{entry.name}</span>
                    <span className="shrink-0 text-xs text-muted-foreground">{formatBytes(entry.size)}</span>
                  </button>
                </li>
              ))}
            </ul>
          )}
          <div className="mt-5 flex justify-end">
            <Button variant="outline" size="sm" onClick={() => setRestoreList(null)}>
              {t('common.cancel')}
            </Button>
          </div>
        </Modal>
      )}

      {restoreTarget && (
        <Modal title={t('webdav.restoreConfirmTitle')} onClose={() => setRestoreTarget(null)} showCloseButton={false}>
          <p className="text-sm text-muted-foreground">
            {t('webdav.restoreConfirmBody', { name: restoreTarget.name })}
          </p>
          <div className="mt-5 flex justify-end gap-2">
            <Button variant="outline" size="sm" onClick={() => setRestoreTarget(null)}>
              {t('common.cancel')}
            </Button>
            <Button size="sm" onClick={() => void handleRestore()}>
              {t('webdav.restoreConfirmOk')}
            </Button>
          </div>
        </Modal>
      )}

      {restoreDone && (
        <Modal title={t('webdav.restoreDoneTitle')} onClose={() => undefined} locked showCloseButton={false}>
          <p className="text-sm text-muted-foreground">{t('webdav.restoreDoneBody')}</p>
        </Modal>
      )}
    </Card>
  )
}
