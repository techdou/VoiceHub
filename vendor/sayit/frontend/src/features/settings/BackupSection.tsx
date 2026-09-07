import { useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Check, Download, FolderOpen, Loader2, Upload } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Tooltip } from '@/components/ui/tooltip'
import {
  ConfigExportDialog,
  ConfigImportDialog,
  FullImportConfirmDialog,
  ImportDoneDialog,
} from './ConfigTransferDialogs'
import {
  exportConfigFile,
  exportFullFile,
  inspectConfigImport,
  pickImportFile,
  runImport,
  restartApp,
  onBackupExportProgress,
  type BackupExportProgress,
  type ConfigExportSelection,
  type ConfigImportPreview,
} from '@/services/backup'
import { formatBytes } from '@/lib/utils'
import type { TranslationKey } from '@/i18n'
import { useT } from '@/i18n/useT'

type ImportKind = 'config' | 'full'

type ImportConfirmation =
  | { kind: 'config'; path: string; preview: ConfigImportPreview }
  | { kind: 'full'; path: string }

type BusyAction = 'exportConfig' | 'exportFull' | 'importConfig' | 'importFull' | null

const phaseLabelKeys: Record<BackupExportProgress['phase'], TranslationKey> = {
  preparing: 'backup.status.preparing',
  packingData: 'backup.status.packingData',
  packingAudio: 'backup.status.packingAudio',
  finalizing: 'backup.status.finalizing',
  completed: 'backup.status.completed',
  failed: 'backup.status.failed',
}

function SavedPath({
  label,
  path,
  onOpen,
  success = false,
}: {
  label: string
  path: string
  onOpen: () => void
  success?: boolean
}) {
  const t = useT()
  return (
    <div className={`mt-2 flex min-w-0 items-center gap-2 rounded-md px-3 py-2 text-xs ${success ? 'bg-success/10 text-success' : 'bg-muted/50 text-muted-foreground'}`}>
      {success && <Check className="h-3.5 w-3.5 shrink-0" />}
      <span className="shrink-0">{label}</span>
      <span className="min-w-0 flex-1 truncate text-foreground/80" title={path}>{path}</span>
      <Tooltip content={t('backup.openFolder')}>
        <button
          type="button"
          onClick={onOpen}
          aria-label={t('backup.openFolder')}
          className="shrink-0 rounded p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        >
          <FolderOpen className="h-3.5 w-3.5" />
        </button>
      </Tooltip>
    </div>
  )
}

function ExportProgress({ progress }: { progress: BackupExportProgress }) {
  const t = useT()
  const percent = Math.max(0, Math.min(100, progress.percent))
  const detail = progress.phase === 'packingAudio' && progress.totalFiles > 0
    ? t('backup.fileProgress', {
      done: progress.processedFiles,
      total: progress.totalFiles,
      doneBytes: formatBytes(progress.processedBytes),
      totalBytes: formatBytes(progress.totalBytes),
    })
    : progress.currentFile || t('backup.pleaseWait')

  return (
    <div className="mt-3 rounded-md border border-border bg-muted/30 px-3 py-2.5">
      <div className="flex items-center justify-between gap-3 text-xs">
        <span className="font-medium text-foreground">{t(phaseLabelKeys[progress.phase])}</span>
        <span className="tabular-nums text-muted-foreground">{Math.round(percent)}%</span>
      </div>
      <div
        className="mt-2 h-1.5 overflow-hidden rounded-full bg-muted"
        role="progressbar"
        aria-label={t('backup.fullProgressAria')}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(percent)}
      >
        <div
          className="h-full rounded-full bg-primary transition-[width] duration-200"
          style={{ width: `${percent}%` }}
        />
      </div>
      <p className="mt-1.5 truncate text-xs text-muted-foreground" title={detail}>{detail}</p>
      <p className="mt-1 truncate text-[11px] text-muted-foreground/80" title={progress.filePath}>
        {t('backup.savedTo', { path: progress.filePath })}
      </p>
    </div>
  )
}

export default function BackupSection() {
  const t = useT()
  const [busyAction, setBusyAction] = useState<BusyAction>(null)
  const [configPath, setConfigPath] = useState('')
  const [fullPath, setFullPath] = useState('')
  const [fullProgress, setFullProgress] = useState<BackupExportProgress | null>(null)
  const [configExportOpen, setConfigExportOpen] = useState(false)
  // 导入确认（配置先预检并展示变更；全部数据保持覆盖确认）
  const [importConfirm, setImportConfirm] = useState<ImportConfirmation | null>(null)
  // 导入成功后展示提示并自动重启
  const [importDone, setImportDone] = useState(false)
  // 导出/导入的错误提示，改为应用内内联横幅
  const [actionError, setActionError] = useState('')

  useEffect(() => {
    const unlisten = onBackupExportProgress((progress) => {
      setFullProgress(progress)
      if (progress.filePath) setFullPath(progress.filePath)
    })
    return () => { void unlisten.then((fn) => fn()) }
  }, [])

  const isBusy = busyAction !== null
  const fullExportRunning = fullProgress?.status === 'running'
  const fullProgressError = useMemo(
    () => fullProgress?.status === 'failed' ? fullProgress.error || t('backup.unknownError') : '',
    [fullProgress, t],
  )

  const revealFile = (filePath: string) => {
    void invoke('reveal_file_in_folder', { filePath })
  }

  const handleExportConfig = async (selection: ConfigExportSelection) => {
    setConfigExportOpen(false)
    setBusyAction('exportConfig')
    setActionError('')
    try {
      const result = await exportConfigFile(selection)
      if (result.filePath) setConfigPath(result.filePath)
    } catch (error) {
      setActionError(t('backup.exportFailed', { message: String(error) }))
    } finally {
      setBusyAction(null)
    }
  }

  const handleExportFull = async () => {
    setBusyAction('exportFull')
    setActionError('')
    setFullProgress(null)
    try {
      const result = await exportFullFile()
      if (result.filePath) setFullPath(result.filePath)
    } catch (error) {
      setActionError(t('backup.exportFailed', { message: String(error) }))
    } finally {
      setBusyAction(null)
    }
  }

  // 第一步：选文件。配置文件先自动识别并计算变更预览，全部数据沿用覆盖确认。
  const handleImport = async (kind: ImportKind) => {
    setBusyAction(kind === 'config' ? 'importConfig' : 'importFull')
    setActionError('')
    try {
      const path = await pickImportFile(kind)
      if (!path) return
      if (kind === 'config') {
        const preview = await inspectConfigImport(path)
        setImportConfirm({ kind, path, preview })
      } else {
        setImportConfirm({ kind, path })
      }
    } catch (error) {
      setActionError(t('backup.importFailed', { message: String(error) }))
    } finally {
      setBusyAction(null)
    }
  }

  // 第二步：确认覆盖后执行导入，成功则展示提示并自动重启
  const confirmImport = async () => {
    if (!importConfirm) return
    const { kind, path } = importConfirm
    const importToken = importConfirm.kind === 'config'
      ? importConfirm.preview.importToken
      : undefined
    setImportConfirm(null)
    setBusyAction(kind === 'config' ? 'importConfig' : 'importFull')
    setActionError('')
    try {
      await runImport(kind, path, importToken)
      setImportDone(true)
      // 略作停留让用户看到提示，再自动重启使更改生效
      setTimeout(() => { void restartApp() }, 1500)
    } catch (error) {
      setActionError(t('backup.importFailed', { message: String(error) }))
      setBusyAction(null)
    }
  }

  return (
    <Card>
      <CardContent className="p-6">
        <h2 className="text-lg font-semibold">{t('backup.title')}</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          {t('backup.desc')}
        </p>

        <div className="mt-5">
          <div className="flex items-center justify-between gap-3">
            <div className="min-w-0">
              <p className="text-sm font-medium">{t('backup.configTitle')}</p>
              <p className="mt-0.5 text-xs text-muted-foreground">{t('backup.configDesc')}</p>
            </div>
            <div className="flex shrink-0 gap-2">
              <Button variant="outline" size="sm" className="gap-1.5" onClick={() => setConfigExportOpen(true)} disabled={isBusy}>
                {busyAction === 'exportConfig' ? <Loader2 className="h-4 w-4 animate-spin" /> : <Download className="h-4 w-4" />}
                {busyAction === 'exportConfig' ? t('backup.exporting') : t('backup.export')}
              </Button>
              <Button variant="outline" size="sm" className="gap-1.5" onClick={() => void handleImport('config')} disabled={isBusy}>
                {busyAction === 'importConfig' ? <Loader2 className="h-4 w-4 animate-spin" /> : <Upload className="h-4 w-4" />}
                {busyAction === 'importConfig' ? t('backup.importing') : t('backup.import')}
              </Button>
            </div>
          </div>
          {configPath && (
            <SavedPath label={t('backup.savedToLabel')} path={configPath} success onOpen={() => revealFile(configPath)} />
          )}
        </div>

        <div className="mt-4 border-t border-border pt-4">
          <div className="flex items-center justify-between gap-3">
            <div className="min-w-0">
              <p className="text-sm font-medium">{t('backup.fullTitle')}</p>
              <p className="mt-0.5 text-xs text-muted-foreground">{t('backup.fullDesc')}</p>
            </div>
            <div className="flex shrink-0 gap-2">
              <Button variant="outline" size="sm" className="gap-1.5" onClick={() => void handleExportFull()} disabled={isBusy}>
                {busyAction === 'exportFull' ? <Loader2 className="h-4 w-4 animate-spin" /> : <Download className="h-4 w-4" />}
                {busyAction === 'exportFull' ? t('backup.packing') : t('backup.export')}
              </Button>
              <Button variant="outline" size="sm" className="gap-1.5" onClick={() => void handleImport('full')} disabled={isBusy}>
                {busyAction === 'importFull' ? <Loader2 className="h-4 w-4 animate-spin" /> : <Upload className="h-4 w-4" />}
                {busyAction === 'importFull' ? t('backup.importing') : t('backup.import')}
              </Button>
            </div>
          </div>

          {fullExportRunning && fullProgress && <ExportProgress progress={fullProgress} />}
          {fullProgress?.status === 'completed' && fullPath && (
            <SavedPath label={t('backup.savedToLabel')} path={fullPath} success onOpen={() => revealFile(fullPath)} />
          )}
          {fullProgressError && (
            <div className="mt-2 rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive">
              {t('backup.exportFailed', { message: fullProgressError })}
            </div>
          )}
        </div>

        {actionError && (
          <div className="mt-4 rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive">
            {actionError}
          </div>
        )}
      </CardContent>

      {configExportOpen && (
        <ConfigExportDialog
          onClose={() => setConfigExportOpen(false)}
          onExport={(selection) => { void handleExportConfig(selection) }}
        />
      )}

      {importConfirm?.kind === 'config' && (
        <ConfigImportDialog
          filePath={importConfirm.path}
          preview={importConfirm.preview}
          onClose={() => setImportConfirm(null)}
          onConfirm={() => { void confirmImport() }}
        />
      )}

      {importConfirm?.kind === 'full' && (
        <FullImportConfirmDialog
          filePath={importConfirm.path}
          onClose={() => setImportConfirm(null)}
          onConfirm={() => { void confirmImport() }}
        />
      )}

      {importDone && <ImportDoneDialog />}
    </Card>
  )
}
