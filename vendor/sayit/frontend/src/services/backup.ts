// 备份 / 恢复：配置（JSON）与全部数据（zip，含音频）的导入导出。
//
// 配置导出支持完整配置或热词组、文本替换、润色模式的自定义选择。
// 配置导入先由 Rust 识别文件并生成变更预览，再由用户二次确认。
// 全量导出与导入保持原有覆盖语义；导入完成后重启应用使运行时状态重载。

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-dialog'
import type { TextReplacementRule } from './textReplacement'
import { t } from '@/i18n'

export interface ExportResult {
  canceled: boolean
  filePath: string | null
}

export type ConfigExportSelection =
  | { mode: 'full' }
  | {
    mode: 'selected'
    hotwordGroupIds: string[]
    includeTextReplacements: boolean
    textReplacements?: TextReplacementRule[]
    promptPresetIds: string[]
  }

export interface ConfigImportSectionPreview {
  kind: string
  total: number
  added: number
  updated: number
  skipped: number
}

export interface ConfigImportWarning {
  code: 'hotwordLimit' | 'fullOverwrite' | string
  current: number | null
  limit: number | null
}

export interface ConfigImportPreview {
  scope: 'full' | 'selected'
  formatVersion: number
  importToken: string
  sections: ConfigImportSectionPreview[]
  warnings: ConfigImportWarning[]
  requiresRestart: boolean
}

export interface ConfigImportResult {
  changedSections: string[]
  added: number
  updated: number
  skipped: number
  requiresRestart: boolean
}

export interface BackupExportProgress {
  status: 'running' | 'completed' | 'failed'
  phase: 'preparing' | 'packingData' | 'packingAudio' | 'finalizing' | 'completed' | 'failed'
  filePath: string
  currentFile: string | null
  processedFiles: number
  totalFiles: number
  processedBytes: number
  totalBytes: number
  percent: number
  error: string | null
}

export function getBackupDirectory(): Promise<string> {
  return invoke<string>('get_backup_directory')
}

export function onBackupExportProgress(
  handler: (progress: BackupExportProgress) => void,
): Promise<UnlistenFn> {
  return listen<BackupExportProgress>('backup-export-progress', (event) => handler(event.payload))
}

/** 导出完整配置，或导出选中的热词组、文本替换和自定义润色模式。 */
export async function exportConfigFile(selection: ConfigExportSelection): Promise<ExportResult> {
  const path = await invoke<string>('export_config', { selection })
  return { canceled: false, filePath: path }
}

/**
 * 本地「全部数据」导出：配置 + 历史 + 录音，直接写入默认备份目录。
 *
 * scope 显式传全 true —— Rust 侧刻意不给这两个字段 serde 默认值，漏传就反序列化失败。
 * 宁可报一条看得见的错，也不要让「默认全都要」在 WebDAV 自动备份那边变成
 * 每天悄悄上传几个 GB 音频。
 */
export async function exportFullFile(): Promise<ExportResult> {
  const path = await invoke<string>('export_full', {
    scope: { includeHistory: true, includeAudio: true },
  })
  return { canceled: false, filePath: path }
}

/** 让用户选择要导入的备份文件；返回路径，取消则返回 null。文件选择走系统原生对话框。 */
export async function pickImportFile(kind: 'config' | 'full'): Promise<string | null> {
  const filters =
    kind === 'config'
      ? [{ name: t('configTransfer.fileConfig'), extensions: ['json'] }]
      : [{ name: t('configTransfer.fileBackup'), extensions: ['zip'] }]
  const picked = await open({ multiple: false, directory: false, filters })
  return typeof picked === 'string' ? picked : null
}

/** 检查配置文件并按当前本地数据计算新增、更新和跳过数量，不写入任何数据。 */
export function inspectConfigImport(inPath: string): Promise<ConfigImportPreview> {
  return invoke<ConfigImportPreview>('inspect_config_import', { inPath })
}

/** 执行已确认的导入；配置返回合并结果，全部数据保持原有流程。 */
export async function runImport(
  kind: 'config' | 'full',
  inPath: string,
  expectedImportToken?: string,
): Promise<ConfigImportResult | null> {
  if (kind === 'config') {
    if (!expectedImportToken) throw new Error(t('configTransfer.missingConfirmation'))
    return invoke<ConfigImportResult>('import_config', { inPath, expectedImportToken })
  }
  await invoke('import_full', { inPath })
  return null
}

/** 重启应用（Tauri 内置 app.restart()）。 */
export function restartApp(): Promise<void> {
  return invoke('restart_app')
}
