// WebDAV 备份的前端封装。
//
// 备份档位（是否含历史/音频）**只存在设置里**，不作为参数传给 Rust：手动「立即备份」
// 和后台定时备份走同一个命令、读同一份设置，界面上勾了什么就一定是上传的内容。
//
// 错误处理约定：Rust 对可预期的用户输入/服务器问题返回大写错误码（WEBDAV_*），
// 其余返回带 HTTP 状态码的英文说明。`describeWebDavError` 把前者翻成本地化文案，
// 后者原样透出——那类信息（状态码、响应体片段）才是排查时真正需要的。

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { t, type TranslationKey } from '@/i18n'

/** 坚果云的 WebDAV 根地址。 */
export const JIANGUOYUN_DAV_BASE = 'https://dav.jianguoyun.com/dav'

/**
 * 地址栏的预填值。
 *
 * **必须是 `/dav` 下的一个子目录，不能是 `/dav` 本身。** 坚果云的 `/dav` 根只列同步
 * 文件夹，往根上 PUT 文件会被拒 —— 各家 WebDAV 客户端的坚果云配置说明都要求先建
 * 一个一级目录再把地址指向它。目录不存在时我们自己 MKCOL 出来，所以用户不用先去
 * 网页上手动建。
 */
export const DEFAULT_DAV_URL = `${JIANGUOYUN_DAV_BASE}/SayIt`

export interface WebDavConfig {
  url: string
  username: string
  password: string
}

export interface WebDavEntry {
  name: string
  size: number
}

export interface WebDavBackupResult {
  fileName: string
  bytes: number
  includeHistory: boolean
  includeAudio: boolean
  finishedAt: number
  /** 本次清理掉的旧备份份数 */
  pruned: number
}

/** 上次备份的结果。失败也会记 —— 静默失败的备份比没有备份更糟。 */
export interface WebDavLastResult {
  at: number
  ok: boolean
  fileName: string
  bytes: number
  includeHistory: boolean
  includeAudio: boolean
  error: string | null
}

export interface WebDavProgress {
  status: 'running' | 'completed' | 'failed'
  phase:
  | 'preparing'
  | 'packingData'
  | 'packingAudio'
  | 'finalizing'
  | 'uploading'
  | 'verifying'
  | 'completed'
  | 'failed'
  fileName: string
  currentFile: string | null
  processedBytes: number
  totalBytes: number
  percent: number
  error: string | null
}

/** Rust 侧返回的错误码 → 文案键。没登记的码按原样显示。 */
const ERROR_KEYS: Record<string, TranslationKey> = {
  WEBDAV_URL_EMPTY: 'webdav.error.urlEmpty',
  WEBDAV_URL_INSECURE: 'webdav.error.urlInsecure',
  WEBDAV_URL_SCHEME: 'webdav.error.urlScheme',
  WEBDAV_CREDENTIALS_EMPTY: 'webdav.error.credentialsEmpty',
  WEBDAV_UNAUTHORIZED: 'webdav.error.unauthorized',
  WEBDAV_MKCOL_CONFLICT: 'webdav.error.mkcolConflict',
  WEBDAV_VERIFY_MISSING: 'webdav.error.verifyMissing',
  WEBDAV_VERIFY_SIZE: 'webdav.error.verifySize',
  WEBDAV_EMPTY_DOWNLOAD: 'webdav.error.emptyDownload',
  WEBDAV_BAD_NAME: 'webdav.error.badName',
}

/**
 * 把 Rust 抛出的错误变成可以直接显示的一句话。
 *
 * invoke 抛出的是字符串本身（Tauri 把 Err(String) 原样传过来），但外面可能已经被
 * 包成 Error，所以两种都要认。
 */
export function describeWebDavError(error: unknown): string {
  const raw = error instanceof Error ? error.message : String(error)
  const code = raw.trim()
  const key = ERROR_KEYS[code]
  return key ? t(key) : raw
}

export function onWebDavBackupProgress(
  handler: (progress: WebDavProgress) => void,
): Promise<UnlistenFn> {
  return listen<WebDavProgress>('webdav-backup-progress', (event) => handler(event.payload))
}

/** 连接测试。返回服务器上已有的备份份数。 */
export function testWebDavConnection(config: WebDavConfig): Promise<number> {
  return invoke<number>('webdav_test', { config })
}

/** 列出服务器上的备份，从新到旧。 */
export function listWebDavBackups(): Promise<WebDavEntry[]> {
  return invoke<WebDavEntry[]>('webdav_list')
}

/** 立即备份一次。档位取自设置。 */
export function runWebDavBackup(): Promise<WebDavBackupResult> {
  return invoke<WebDavBackupResult>('webdav_backup_now')
}

/** 从服务器恢复。完成后需要由调用方触发重启（与本地导入一致）。 */
export function restoreWebDavBackup(name: string): Promise<void> {
  return invoke('webdav_restore', { name })
}
