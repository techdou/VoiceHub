// WebDAV 自动备份的调度。
//
// 结构照着 features/update/autoUpdate.ts 抄：模块级 inFlight 锁 + setInterval + 一个
// 小状态store。项目里只有这一种周期任务 pattern，别再引第二种。
//
// 三条约束值得单独说明：
//
// 1. **判据是时间戳，不是 tick 计数。** setInterval 只在应用运行期计时，休眠和关机
//    期间不补偿。按 tick 数算「到 24 小时了吗」，一台每天只开两小时的机器要 12 天
//    才备份一次。所以每次 tick 都拿 `Date.now()` 和上次成功时间比。
// 2. **失败后要退避。** 坚果云对 WebDAV 有请求频率限制，地址填错时每 15 分钟重试
//    一次除了刷日志没有别的作用，还可能把账号打进限流。
// 3. **enabled 每次 tick 都重读。** 用户在设置里打开开关后不该要求重启应用。

import { getSetting, setSetting } from '@/services/store'
import { addRuntimeEvent } from '@/services/debugLog'
import {
  runWebDavBackup,
  type WebDavBackupResult,
  type WebDavLastResult,
} from '@/services/webdavBackup'

/** 上次成功备份的时刻（ms）。间隔判定只看这个。 */
const KEY_LAST_BACKUP_AT = 'webdav.lastBackupAt'
/** 上次尝试的时刻（ms），成功失败都记。失败退避用它。 */
const KEY_LAST_ATTEMPT_AT = 'webdav.lastAttemptAt'

/**
 * 轮询周期。它**不是**备份间隔 —— 备份间隔由 webdav.intervalHours 决定，这里只是
 * 「多久去看一眼该不该备份了」。15 分钟足够让「刚过 24 小时」这件事被及时发现，
 * 每次开销是读三条设置。
 */
const TICK_MS = 15 * 60 * 1000

/**
 * 启动后第一次检查的延迟。
 *
 * 不在挂载时立刻跑：那正是加载本地模型、恢复窗口、检查更新最挤的时候，一个可能
 * 要压几百 MB 的打包挤进去只会让启动更慢。两分钟后再看，用户感知不到差别。
 */
const FIRST_CHECK_DELAY_MS = 2 * 60 * 1000

/** 失败后至少等这么久再试，避免撞上网盘的请求频率限制。 */
const FAILURE_BACKOFF_MS = 60 * 60 * 1000

export interface WebDavBackupState {
  /** 正在备份。UI 用它禁用按钮、显示进度。 */
  running: boolean
  lastResult: WebDavLastResult | null
}

let currentState: WebDavBackupState = { running: false, lastResult: null }
const listeners = new Set<(state: WebDavBackupState) => void>()
/** 正在进行中的备份，防止定时任务和用户手动点击撞在一起传两份。 */
let inFlight: Promise<WebDavBackupResult | null> | null = null
let tickTimer: ReturnType<typeof setInterval> | null = null
let firstCheckTimer: ReturnType<typeof setTimeout> | null = null

function setState(patch: Partial<WebDavBackupState>) {
  currentState = { ...currentState, ...patch }
  listeners.forEach((cb) => cb(currentState))
}

export function getWebDavBackupState(): WebDavBackupState {
  return currentState
}

export function onWebDavBackupChange(cb: (state: WebDavBackupState) => void) {
  listeners.add(cb)
  return () => { listeners.delete(cb) }
}

/** 从设置读回上次结果（Rust 侧在每次备份后写入）。 */
export async function refreshLastResult(): Promise<WebDavLastResult | null> {
  const value = await getSetting<WebDavLastResult | null>('webdav.lastResult', null).catch(() => null)
  const lastResult = value && typeof value === 'object' ? value : null
  setState({ lastResult })
  return lastResult
}

/**
 * 跑一次备份。手动按钮和定时任务共用。
 *
 * 无论成功失败都写 lastAttemptAt，成功才写 lastBackupAt —— 失败不推进备份时间，
 * 否则一次失败会让下一次备份被推迟整个间隔。
 */
export async function runBackupNow(trigger: 'manual' | 'schedule'): Promise<WebDavBackupResult | null> {
  if (inFlight) return inFlight

  const task = (async (): Promise<WebDavBackupResult | null> => {
    setState({ running: true })
    const startedAt = Date.now()
    await setSetting(KEY_LAST_ATTEMPT_AT, startedAt).catch(() => undefined)
    try {
      const result = await runWebDavBackup()
      await setSetting(KEY_LAST_BACKUP_AT, Date.now()).catch(() => undefined)
      addRuntimeEvent('info', 'webdav', 'backup finished', {
        trigger,
        fileName: result.fileName,
        bytes: result.bytes,
        includeHistory: result.includeHistory,
        includeAudio: result.includeAudio,
        pruned: result.pruned,
        elapsedMs: Date.now() - startedAt,
      })
      return result
    } catch (error) {
      // 失败必须留声，而且要能和「没开启备份」区分开。一个静默失败几个月的备份
      // 在界面上和一个正常工作的备份长得一模一样。
      addRuntimeEvent('error', 'webdav', 'backup failed', { trigger, error: String(error) })
      return null
    }
  })()

  inFlight = task
  try {
    return await task
  } finally {
    inFlight = null
    setState({ running: false })
    await refreshLastResult()
  }
}

/** 到点了吗。读设置而不是缓存，用户改了间隔立刻生效。 */
async function shouldBackupNow(): Promise<boolean> {
  const enabled = await getSetting('webdav.enabled', false).catch(() => false)
  if (!enabled) return false

  // 地址或凭证没配完就别去打包：打完才发现传不上去纯属白烧一次 CPU 和磁盘。
  const [url, username, password] = await Promise.all([
    getSetting('webdav.url', '').catch(() => ''),
    getSetting('webdav.username', '').catch(() => ''),
    getSetting('webdav.password', '').catch(() => ''),
  ])
  if (!url.trim() || !username.trim() || !password) return false

  const now = Date.now()
  const lastAttemptAt = await getSetting(KEY_LAST_ATTEMPT_AT, 0).catch(() => 0)
  const lastBackupAt = await getSetting(KEY_LAST_BACKUP_AT, 0).catch(() => 0)

  // 上次尝试失败（尝试比成功新）→ 走退避，别每 15 分钟撞一次限流。
  if (lastAttemptAt > lastBackupAt && now - lastAttemptAt < FAILURE_BACKOFF_MS) return false

  const intervalHours = await getSetting('webdav.intervalHours', 24).catch(() => 24)
  const intervalMs = Math.max(1, intervalHours) * 60 * 60 * 1000
  // lastBackupAt 为 0 = 从没成功备份过，开启后第一次检查就该跑。
  return now - lastBackupAt >= intervalMs
}

async function tick() {
  try {
    if (await shouldBackupNow()) {
      await runBackupNow('schedule')
    }
  } catch (error) {
    // tick 自己不能抛：它跑在 setInterval 里，抛出去就是一个没人接的
    // unhandled rejection，而且下一次 tick 照旧 —— 表现成毫无痕迹。
    addRuntimeEvent('error', 'webdav', 'backup tick threw', { error: String(error) })
  }
}

/**
 * 启动自动备份服务。在 App.tsx 挂载时调用一次。
 *
 * 即使当前没开启也会起 ticker：开关是每次 tick 重读的，这样用户在设置里打开后
 * 不需要重启应用。代价是每 15 分钟读一次设置。
 */
export async function startWebDavBackupService(): Promise<void> {
  // 整个函数包一层：调用方是 `void startWebDavBackupService()`，异常会被 Promise
  // 静默吞掉，表现成「自动备份从来没跑过」且不留痕迹。
  try {
    await refreshLastResult()
    const enabled = await getSetting('webdav.enabled', false).catch(() => false)
    // 无条件记一条：排查时先看这行在不在，不在就说明服务压根没启动。
    addRuntimeEvent('info', 'webdav', 'backup service starting', {
      enabled,
      intervalHours: await getSetting('webdav.intervalHours', 24).catch(() => 24),
      includeHistory: await getSetting('webdav.includeHistory', false).catch(() => false),
      includeAudio: await getSetting('webdav.includeAudio', false).catch(() => false),
      lastBackupAt: await getSetting(KEY_LAST_BACKUP_AT, 0).catch(() => 0),
    })

    if (firstCheckTimer === null) {
      firstCheckTimer = setTimeout(() => { void tick() }, FIRST_CHECK_DELAY_MS)
    }
    if (tickTimer === null) {
      tickTimer = setInterval(() => { void tick() }, TICK_MS)
    }
  } catch (error) {
    addRuntimeEvent('error', 'webdav', 'backup service failed to start', { error: String(error) })
  }
}
