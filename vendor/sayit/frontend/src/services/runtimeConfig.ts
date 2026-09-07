import * as bridge from './bridge'
import { t } from '@/i18n'

declare const __SAYIT_DEFAULT_SERVER_URL__: string

const BUILTIN_DEFAULT_SERVER_URL =
  typeof __SAYIT_DEFAULT_SERVER_URL__ === 'string' && __SAYIT_DEFAULT_SERVER_URL__.trim()
    ? __SAYIT_DEFAULT_SERVER_URL__.trim()
    : 'https://sayitapp.site'

const BACKEND_BASE_URL_STORE_KEY = 'backendBaseUrl'

interface RuntimeEnv {
  VITE_BACKEND_BASE_URL?: string
  VITE_WS_URL?: string
  DEV?: boolean
}

function getEnv(): RuntimeEnv {
  return (import.meta as unknown as { env?: RuntimeEnv }).env || {}
}

function trimSlash(url: string): string {
  return url.replace(/\/+$/, '')
}

function normalizeUrl(value: string | null | undefined): string {
  return trimSlash(String(value || '').trim())
}

function resolveBuiltinDefaultBaseUrl(): string {
  const env = getEnv()
  const value = normalizeUrl(env.VITE_BACKEND_BASE_URL)
  if (value) return value
  return trimSlash(BUILTIN_DEFAULT_SERVER_URL)
}

function resolveEnvOverrideBaseUrl(): string {
  const env = getEnv()
  return normalizeUrl(env.VITE_BACKEND_BASE_URL)
}

let backendBaseUrl = resolveBuiltinDefaultBaseUrl()

export async function initRuntimeConfig(): Promise<void> {
  // 用户主动保存的地址优先于环境变量
  const stored = await bridge.storeGet(BACKEND_BASE_URL_STORE_KEY)
  const normalized = normalizeUrl(typeof stored === 'string' ? stored : '')
  if (normalized) {
    backendBaseUrl = normalized
    return
  }
  const envOverride = resolveEnvOverrideBaseUrl()
  if (envOverride) {
    backendBaseUrl = envOverride
    return
  }
  backendBaseUrl = resolveBuiltinDefaultBaseUrl()
}

export function getDefaultBackendBaseUrl(): string {
  return resolveBuiltinDefaultBaseUrl()
}

export function getBackendBaseUrl(): string {
  return backendBaseUrl
}

/**
 * 检查更新时用的地址 —— **跟随** getBackendBaseUrl()：服务器地址填哪个，就找哪个要更新。
 *
 * 这么定是为了「填一个地址就能整体切到测试环境」这件事足够简单，不用第二个设置项。
 * 代价与配套约束（改这里之前先看 .kiro/decisions.md 的「更新通道」一节）：
 *  · 自建后端的用户，他的服务器上没有 manifest。**所以 checkVersionUpdate 必须保留
 *    回落到官方地址的那一步** —— 没有它，这些用户会拿到 404、被静默当成"没有新版"，
 *    从此永远收不到更新。那一步不是优化，是这个设计能成立的前提。
 *  · 更新包没有签名（main.rs 的 updater 插件因此禁用），manifest 的 SHA-512 只证明
 *    "下载的字节与 manifest 一致"、不证明"这个包是我们发的"。也就是说这个地址同时
 *    决定了「谁能在这台机器上装程序」。因此非官方地址时，关于页必须把更新来源显示
 *    出来（见 isOfficialUpdateChannel 的用处），别让它悄无声息。
 */
export function getUpdateBaseUrl(): string {
  return getBackendBaseUrl()
}

/** 内置的官方更新地址。回落用，也用于判断当前是否偏离了官方通道。 */
export function getOfficialUpdateBaseUrl(): string {
  return resolveBuiltinDefaultBaseUrl()
}

/**
 * 更新是否来自官方地址。
 * 为 false 时关于页会把实际来源显示出来 —— 一台从别处取更新的机器如果看起来和
 * 正常机器一样，早晚会被当成正常机器用。
 */
export function isOfficialUpdateChannel(): boolean {
  return normalizeUrl(getUpdateBaseUrl()) === normalizeUrl(getOfficialUpdateBaseUrl())
}

export async function setBackendBaseUrl(value: string): Promise<string> {
  const normalized = normalizeUrl(value)
  if (!normalized) {
    throw new Error(t('runtimeConfig.emptyServerUrl'))
  }
  backendBaseUrl = normalized
  await bridge.storeSet(BACKEND_BASE_URL_STORE_KEY, normalized)
  return backendBaseUrl
}

export async function resetBackendBaseUrl(): Promise<string> {
  backendBaseUrl = resolveBuiltinDefaultBaseUrl()
  await bridge.storeDelete(BACKEND_BASE_URL_STORE_KEY)
  return backendBaseUrl
}

export async function getStoredBackendBaseUrl(): Promise<string> {
  const stored = await bridge.storeGet(BACKEND_BASE_URL_STORE_KEY)
  return normalizeUrl(typeof stored === 'string' ? stored : '')
}

export function getWSUrl(): string {
  const env = getEnv()
  const explicit = normalizeUrl(env.VITE_WS_URL)
  if (explicit) return explicit

  const base = getBackendBaseUrl()
  if (base.startsWith('https://')) return `${base.replace(/^https:\/\//, 'wss://')}/ws/transcribe`
  if (base.startsWith('http://')) return `${base.replace(/^http:\/\//, 'ws://')}/ws/transcribe`
  return `${resolveBuiltinDefaultBaseUrl().replace(/^https?:\/\//, 'wss://')}/ws/transcribe`
}
