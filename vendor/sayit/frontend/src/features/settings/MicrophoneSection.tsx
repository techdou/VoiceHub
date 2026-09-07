import { useMemo } from 'react'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Select } from '@/components/ui/select'
import { cleanMicLabel } from './utils'
import type { TranslationKey } from '@/i18n'
import { useT } from '@/i18n/useT'

export type MicVolumeLevel = 'idle' | 'silent' | 'low' | 'normal'

/** 存 key 而不是文案：这是模块级常量，只求值一次，存好的中文串切语言时不会变。 */
const VOLUME_CONFIG: Record<MicVolumeLevel, { labelKey: TranslationKey | null; color: string; descKey: TranslationKey | null }> = {
  idle: { labelKey: null, color: '', descKey: null },
  silent: { labelKey: 'mic.level.silent', color: 'text-destructive', descKey: 'mic.level.silentDesc' },
  low: { labelKey: 'mic.level.low', color: 'text-amber-500', descKey: 'mic.level.lowDesc' },
  normal: { labelKey: 'mic.level.normal', color: 'text-emerald-500', descKey: 'mic.level.normalDesc' },
}

export default function MicrophoneSection({
  mics,
  selectedMic,
  testing,
  volumeLevel,
  errorMessage,
  onCanvasRef,
  onMicChange,
  onTestMic,
}: {
  mics: MediaDeviceInfo[]
  selectedMic: string
  testing: boolean
  volumeLevel: MicVolumeLevel
  errorMessage?: string
  onCanvasRef: (node: HTMLCanvasElement | null) => void
  onMicChange: (deviceId: string) => void
  onTestMic: () => void
}) {
  const t = useT()
  const micOptions = useMemo(() => {
    return [
      { value: '', label: t('mic.systemDefault') },
      ...mics.map((mic) => ({
        value: mic.deviceId,
        // 设备名来自系统，不翻译；只有"读不到名字"时的兜底标签跟界面语言。
        label: cleanMicLabel(mic.label) || t('mic.unnamed', { id: mic.deviceId.slice(0, 8) }),
      })),
    ]
  }, [mics, t])

  const vol = VOLUME_CONFIG[volumeLevel]

  return (
    <Card>
      <CardContent className="p-6">
        <h2 className="mb-4 text-lg font-semibold">{t('mic.title')}</h2>
        <div className="space-y-4">
          <div>
            <label className="mb-2 block text-sm text-foreground">{t('mic.select')}</label>
            <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
              <Select
                value={selectedMic}
                onChange={onMicChange}
                options={micOptions}
                className="sm:flex-1"
              />
              <Button variant="outline" size="sm" onClick={onTestMic} disabled={testing} className="h-9 shrink-0 px-4">
                {testing ? t('mic.testing') : t('mic.test')}
              </Button>
            </div>
          </div>

          {testing && (
            <div className="space-y-2">
              <canvas
                ref={onCanvasRef}
                width={160}
                height={40}
                className="mx-auto rounded-md border border-border"
                style={{ width: '160px', height: '40px' }}
              />
              {volumeLevel !== 'idle' && (
                <div className="text-center">
                  <span className={`text-xs font-medium ${vol.color}`}>{vol.labelKey ? t(vol.labelKey) : ''}</span>
                  <p className="mt-0.5 text-xs text-muted-foreground">{vol.descKey ? t(vol.descKey) : ''}</p>
                </div>
              )}
            </div>
          )}

          {errorMessage && !testing && (
            <p className="text-xs text-destructive">{errorMessage}</p>
          )}
        </div>
      </CardContent>
    </Card>
  )
}
