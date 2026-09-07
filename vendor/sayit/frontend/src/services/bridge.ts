/**
 * Tauri IPC Bridge — 所有前端代码通过这个模块与 Rust 后端通信。
 */

import { invoke } from '@tauri-apps/api/core'
import { listen, emit } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import type { DiagnosticOccurrence, DiagnosticsPreview } from '../types/appApi'

// Re-export for convenience
export { invoke, listen, emit }

// ─── Window Controls ───

export function minimize() {
  getCurrentWindow().minimize()
}

export function maximize() {
  getCurrentWindow().toggleMaximize()
}

export function close() {
  getCurrentWindow().close()
}

// ─── Overlay ───

/**
 * Bug 003 diagnostic helper — log overlay IPC failures to runtime events
 * (which mirror to sayit.log via appendDebugLog).
 */
function logOverlayIpcError(op: string, err: unknown) {
  try {
    // Lazy import to avoid circular dep with debugLog
    void import('./debugLog').then(({ addRuntimeEvent }) => {
      addRuntimeEvent('error', 'overlay-ipc', `${op} failed`, { error: String(err) })
    })
  } catch {
    console.error(`[overlay-ipc] ${op} failed:`, err)
  }
}

export interface OverlayHealthSnapshot {
  showId: number
  activeShowId: number
  activeGeneration: number
  acked: boolean
  ackGeneration: number
  ackLatencyMs: number
  recoveryStarted: boolean
  recoverySucceeded: boolean
  window: {
    handleExists: boolean
    visible?: boolean | null
    intersectsPrimary?: boolean
    position?: { x: number; y: number } | null
    size?: { width: number; height: number } | null
  }
}

/** 原子地更新状态并显示悬浮窗，返回本次显示的关联 id。 */
export function presentOverlay(data: unknown) {
  return invoke<number>('present_overlay', { data }).catch((err) => {
    logOverlayIpcError('present_overlay', err)
    return 0
  })
}

/** 兼容旧调用；新显示流程应优先使用 presentOverlay。 */
export function showOverlay() {
  return invoke<number>('show_overlay').catch((err) => {
    logOverlayIpcError('show_overlay', err)
    return 0
  })
}

export function hideOverlay() {
  return invoke<void>('hide_overlay').catch((err) => {
    logOverlayIpcError('hide_overlay', err)
  })
}

export function updateOverlay(data: unknown) {
  return invoke<void>('update_overlay_state', { data }).catch((err) => {
    logOverlayIpcError('update_overlay_state', err)
  })
}

export function overlayReady() {
  return invoke<void>('overlay_ready')
}

export function overlayRenderAck(data: unknown) {
  return invoke<void>('overlay_render_ack', { data })
}

export function getOverlayHealth(showId: number) {
  return invoke<OverlayHealthSnapshot>('get_overlay_health', { showId })
}

export type EscapeActionMode = 'off' | 'cancel_recording' | 'cancel_processing' | 'dismiss_fallback'

/** 只在悬浮窗需要响应全局 Esc 时开启；Rust 侧带超时保险，避免异常后永久吞键。 */
export function setEscapeActionMode(mode: EscapeActionMode, token = 0) {
  return invoke<void>('set_escape_action_mode', { mode, token })
}

// ─── Paste / Context ───

export function pasteText(text: string, hwnd?: string, focusHwnd?: string, restoreClipboard?: boolean) {
  return invoke<{
    ok: boolean
    strategy?: string
    reason?: string
    detail?: string
    attempts?: Array<{ strategy: string; ok: boolean; reason?: string; detail?: string }>
  }>('paste_text', { text, hwnd: hwnd || null, focusHwnd: focusHwnd || null, restoreClipboard: restoreClipboard ?? false })
}

export function getProbeResult() {
  return invoke<Record<string, unknown>>('get_probe_result')
}

/** 一次原生捕获同时返回录音目标上下文与插字探测结果。 */
export function getRecordingContext(includeTextContext = false) {
  return invoke<{
    appContext: Record<string, unknown>
    probe: Record<string, unknown>
  }>('get_recording_context', { includeTextContext })
}

export function getActiveAppContext() {
  return invoke<Record<string, unknown> | null>('get_active_app_context')
}

export function getClientRuntimeInfo() {
  return invoke<{
    userId: string
    userName: string
    deviceId: string
    hostname: string
    clientVersion: string
    platform: string
    osVersion: string
    localIp: string
    systemLocale: string
    cpuCores: number
    memoryMb: number
  }>('get_client_runtime_info')
}

/**
 * 系统**显示语言**，返回 `zh-CN` / `en`。
 *
 * 与 `getClientRuntimeInfo().systemLocale` 不是一回事：那个是**区域格式**
 * （日期货币怎么写），诊断用；这个是 Windows 界面本身的语言，决定界面语言。
 * 判定只在 Rust 一处（`locale::system_ui_lang`），托盘和界面才不会各说一套。
 */
export function getSystemUiLanguage() {
  return invoke<string>('get_system_ui_language')
}

export function copyText(text: string) {
  return invoke('copy_text', { text })
}

// ─── 系统输出静音（录音期间防回采）───

/** 记录当前默认输出设备静音状态并将其静音。返回是否已处理。 */
export function muteSystemOutput() {
  return invoke<boolean>('mute_system_output')
}

/** 恢复到 muteSystemOutput 之前记录的静音状态。 */
export function restoreSystemOutput() {
  return invoke<boolean>('restore_system_output')
}

export function appendDebugLog(payload: unknown) {
  invoke('append_debug_log', { payload })
}

// ─── Store ───

export function storeGet(key: string) {
  return invoke<unknown>('store_get', { key })
}

export function storeSet(key: string, value: unknown) {
  return invoke('store_set', { key, value })
}

export function storeDelete(key: string) {
  return invoke('store_delete', { key })
}

// ─── History ───

export function historyList(query?: {
  keyword?: string
  favoriteOnly?: boolean
  limit?: number
  offset?: number
}) {
  return invoke<unknown[]>('history_list', { query })
}

export function historyCount(query?: {
  keyword?: string
  favoriteOnly?: boolean
}) {
  return invoke<number>('history_count', { query })
}

export function historyAdd(record: unknown) {
  return invoke('history_add', { record })
}

export function historyUpdate(id: string, patch: Record<string, unknown>) {
  return invoke('history_update', { id, patch })
}

export function historyDelete(id: string) {
  return invoke('history_delete', { id })
}

export function historySetFavorite(id: string, favorite: boolean) {
  return invoke('history_set_favorite', { id, favorite })
}

// ─── Export ───

export function saveTextExport(payload: {
  defaultPath: string
  content: string
  filters?: Array<{ name: string; extensions: string[] }>
}) {
  return invoke<string | null>('save_text_export', { payload })
}

export function saveExportBundle(payload: {
  defaultPath: string
  files: Array<{ name: string; content: string }>
}) {
  return invoke<string | null>('save_export_bundle', { payload })
}

// ─── Shortcuts ───

/** 前端内部广播：快捷键设置已变化，供页面（如首页提示）实时刷新显示。 */
export const SHORTCUTS_CHANGED_EVENT = 'sayit:shortcuts-changed'

export function notifyShortcutsChanged() {
  invoke('shortcuts_changed')
  // 同时在前端广播，让依赖快捷键显示的页面无需切换路由即可刷新
  if (typeof window !== 'undefined') {
    window.dispatchEvent(new CustomEvent(SHORTCUTS_CHANGED_EVENT))
  }
}

export function testShortcut(accelerator: string) {
  return invoke<boolean>('test_shortcut', { accelerator })
}

/** 查询 PTT DOM code 对应按键当前是否真实物理按下（顺序与输入一致）。 */
export function getPTTPhysicalKeyStates(codes: string[]) {
  return invoke<boolean[]>('get_ptt_physical_key_states', { codes })
}

/**
 * 开始快捷键录制捕获（设置页点开录制时调用）。Rust 侧会：
 * - 底层鼠标钩子吞掉下一个侧键并通过 mouse-shortcut-captured 回报；
 * - 键盘钩子在录制期间放行 PTT/免提单键，不触发录音；
 * - 注销全部 global_shortcut，避免已绑定的组合键被系统抢走、录不进来。
 */
export function beginShortcutCapture() {
  return invoke('begin_shortcut_capture').catch(() => { })
}

/** 结束快捷键录制捕获（录制结束/取消时调用），并恢复 Rust 侧的热键注册。 */
export function endShortcutCapture() {
  return invoke('end_shortcut_capture').catch(() => { })
}

// ─── System ───

export function getAutoLaunch() {
  return invoke<boolean>('get_auto_launch')
}

export function setAutoLaunch(enable: boolean) {
  return invoke('set_auto_launch', { enable })
}

/**
 * 拉起安装程序并退出应用。
 * relaunch=true 装完自动重开（用户主动点更新时）；退出路径上的兜底安装走 Rust 内部，
 * 传 false —— 用户是要关掉 SayIt，装完再拉起来会表现成"这软件关不掉"。
 */
export function installDownloadedUpdate(filePath: string, relaunch: boolean) {
  return invoke('install_downloaded_update', { filePath, relaunch })
}

/** 下载安装包到临时目录，返回完整路径。传了 sha512（Base64）就由 Rust 侧校验完整性。 */
export function downloadUpdate(url: string, sha512?: string | null) {
  return invoke<string>('download_update', { url, sha512: sha512 ?? null })
}

/** 磁盘上那个安装包还能不能用（存在 + 哈希对得上）。用于启动时复用上次下载的包。 */
export function verifyUpdatePackage(filePath: string, sha512?: string | null) {
  return invoke<boolean>('verify_update_package', { filePath, sha512: sha512 ?? null })
}

// ─── Tray ───

/**
 * 把托盘右键菜单里「AI 整理」那一项刷成给定状态（文字上的开/关 + 状态图标）。
 *
 * 托盘菜单只在启动时建一次（Tauri v2 没有「菜单即将弹出」的钩子），
 * 所以界面里每次切开关都要主动回写，否则右键看到的是上一次的状态。
 */
export function setTrayAiEnabled(enabled: boolean) {
  return invoke<void>('set_tray_ai_enabled', { enabled }).catch(() => { })
}

/** 托盘右键切换了「AI 整理」（Rust 已落库，payload 是切换后的值）。 */
export function onAiCleanupChanged(cb: (enabled: boolean) => void) {
  const unlisten = listen<{ enabled?: boolean }>('ai-cleanup-changed', (event) => {
    cb(Boolean(event.payload?.enabled))
  })
  return () => { unlisten.then((fn) => fn()) }
}

/** AI 整理快捷键被触发；前端负责更新内存、持久化和悬浮窗提示。 */
export function onAiCleanupToggleRequested(cb: () => void) {
  const unlisten = listen('toggle-ai-cleanup', () => cb())
  return () => { unlisten.then((fn) => fn()) }
}

export function setPTTLabConfig(data: unknown) {
  console.log('[bridge] setPTTLabConfig called', data)
  invoke('set_ptt_lab_config', { data }).catch((err) => {
    console.error('[bridge] setPTTLabConfig failed:', err)
  })
}

// ─── Audio Files ───

export function saveAudioFile(id: string, wavBase64: string) {
  return invoke<string>('save_audio_file', { id, wavBase64 })
}

export function savePcmAsWav(id: string, pcmBase64: string, sampleRate?: number) {
  return invoke<string>('save_pcm_as_wav', { id, pcmBase64, sampleRate: sampleRate ?? null })
}

export function readAudioFile(filePath: string) {
  return invoke<string | null>('read_audio_file', { filePath })
}

/**
 * 录音文件是否还在。
 *
 * 保留期到了之后 Rust 只删文件、不清历史记录里的 audioFilePath，
 * 所以「记录里有路径」不等于「文件还在」，凡是依赖录音的入口都要先问这一句。
 */
export function audioFileExists(filePath: string) {
  return invoke<boolean>('audio_file_exists', { filePath })
}

export function deleteAudioFile(filePath: string) {
  return invoke('delete_audio_file', { filePath })
}

// ─── Diagnostics ───

export function collectSettings() {
  return invoke<Record<string, unknown>>('collect_settings')
}

export function getDiagnosticsPreview(data: {
  settings: Record<string, unknown>
  issueOccurrence: DiagnosticOccurrence
}) {
  return invoke<DiagnosticsPreview>('get_diagnostics_preview', { data })
}

export function createDiagnosticsZip(data: unknown) {
  return invoke<string>('create_diagnostics_zip', { data })
}

export function readDiagnosticsZip(path: string) {
  return invoke<number[] | null>('read_diagnostics_zip', { path })
}

export function copyDiagnosticsZip(source: string, destination: string) {
  return invoke<void>('copy_diagnostics_zip', { source, destination })
}

export function readLogFile(logType: string) {
  return invoke<string | null>('read_log_file', { logType })
}

export function openLogFolder() {
  return invoke('open_log_folder')
}

// ─── Event Listeners ───

export function onOverlayState(cb: (data: unknown) => void) {
  const unlisten = listen<unknown>('overlay-state', (event) => cb(event.payload))
  return () => { unlisten.then((fn) => fn()) }
}

export function onActiveAppContext(cb: (data: unknown) => void) {
  const unlisten = listen<unknown>('active-app-context', (event) => cb(event.payload))
  return () => { unlisten.then((fn) => fn()) }
}

export function onPTTDown(cb: (data?: unknown) => void) {
  const unlisten = listen<unknown>('ptt-down', (event) => cb(event.payload))
  return () => { unlisten.then((fn) => fn()) }
}

export function onPTTUp(cb: (data?: unknown) => void) {
  const unlisten = listen<unknown>('ptt-up', (event) => cb(event.payload))
  return () => { unlisten.then((fn) => fn()) }
}

export function onPTTToggle(cb: (data?: unknown) => void) {
  const unlisten = listen<unknown>('ptt-toggle', (event) => cb(event.payload))
  return () => { unlisten.then((fn) => fn()) }
}

export function onPTTTimeoutWarning(cb: (data?: unknown) => void) {
  const unlisten = listen<unknown>('ptt-timeout-warning', (event) => cb(event.payload))
  return () => { unlisten.then((fn) => fn()) }
}

export function onToggleHandsFree(cb: (data?: unknown) => void) {
  const unlisten = listen<unknown>('toggle-hands-free', (event) => cb(event.payload))
  return () => { unlisten.then((fn) => fn()) }
}

export function onEscapeAction(cb: (data: { mode: EscapeActionMode; token: number }) => void) {
  const unlisten = listen<{ mode: EscapeActionMode; token: number }>('escape-action', (event) => cb(event.payload))
  return () => { unlisten.then((fn) => fn()) }
}

export function onMouseShortcutCaptured(cb: (data: { setting: string; vk: number }) => void) {
  const unlisten = listen<{ setting: string; vk: number }>('mouse-shortcut-captured', (event) => cb(event.payload))
  return () => { unlisten.then((fn) => fn()) }
}

export function onPTTLabEvent(cb: (data?: unknown) => void) {
  const unlisten = listen<unknown>('ptt-lab-event', (event) => cb(event.payload))
  return () => { unlisten.then((fn) => fn()) }
}
