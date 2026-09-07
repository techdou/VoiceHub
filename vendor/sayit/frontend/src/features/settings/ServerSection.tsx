// 服务器模式配置 — 服务地址 + 连接状态

import { useEffect, useState } from 'react'
import { Info } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Tooltip } from '@/components/ui/tooltip'
import { Feedback, type FeedbackTone } from '@/components/ui/feedback'
import { Segmented } from '@/components/ui/segmented'
import {
  getBackendBaseUrl,
  getDefaultBackendBaseUrl,
  resetBackendBaseUrl,
  setBackendBaseUrl as persistBackendBaseUrl,
} from '@/services/runtimeConfig'
import { reconnectProvider } from '@/services/recorder'
import { checkForUpdateNow, discardPendingForChannelSwitch } from '@/features/update/autoUpdate'
import { getSetting, setSetting } from '@/services/store'
import { setEngineDraftDirty } from '@/stores/engineDraft'
import { describeServerError } from '@/lib/errorMessages'
import { t } from '@/i18n'
import { useT } from '@/i18n/useT'

interface ServiceResult {
  tone: FeedbackTone
  message: string
  detail?: string
}

export default function ServerSection() {
  const t = useT()
  const [backendBaseUrl, setBackendBaseUrl] = useState('')
  /** 已保存的地址。输入框与它不一致就是「未保存」 */
  const [savedBaseUrl, setSavedBaseUrl] = useState('')
  const [defaultBaseUrl, setDefaultBaseUrl] = useState('')
  const [result, setResult] = useState<ServiceResult | null>(null)
  const [busy, setBusy] = useState(false)
  const [asrLanguage, setAsrLanguage] = useState('auto')

  useEffect(() => {
    const current = getBackendBaseUrl()
    setBackendBaseUrl(current)
    setSavedBaseUrl(current)
    setDefaultBaseUrl(getDefaultBackendBaseUrl())
    void getSetting('server.language', 'auto').then((v) => setAsrLanguage(String(v || 'auto')))
    // 切走路由时把"有未保存改动"复位，别把脏状态留给下一次进入
    return () => setEngineDraftDirty(false)
  }, [])

  const normalize = (v: string) => v.trim().replace(/\/+$/, '')

  const isDirty = normalize(backendBaseUrl) !== normalize(savedBaseUrl)
  const isCustom = normalize(savedBaseUrl) !== normalize(defaultBaseUrl)

  function handleUrlChange(value: string) {
    setBackendBaseUrl(value)
    setResult(null)
    setEngineDraftDirty(normalize(value) !== normalize(savedBaseUrl))
  }

  /** 探一次 /healthz。成功返回后端上报的 ASR/LLM 开关，失败抛出原始异常。 */
  async function probeHealth(url: string): Promise<{ asr?: boolean; llm?: boolean }> {
    const response = await fetch(`${url}/healthz`, { cache: 'no-store' })
    if (!response.ok) throw new Error(`HTTP ${response.status}`)
    return await response.json() as { asr?: boolean; llm?: boolean }
  }

  /** 把 /healthz 的 asr/llm 布尔量翻译成人话。原来直接显示「ASR=on，LLM=off」，
   *  那是把后端的 JSON 字段原样贴给用户。 */
  function describeHealth(payload: { asr?: boolean; llm?: boolean }, prefix: string): ServiceResult {
    if (payload.asr === false) {
      return {
        tone: 'warning',
        message: t('server.asrNotReady', { prefix }),
      }
    }
    if (payload.llm === false) {
      return {
        tone: 'success',
        message: t('server.noAi', { prefix }),
      }
    }
    return { tone: 'success', message: t('server.allGood', { prefix }) }
  }

  /**
   * 保存并测试。
   *
   * 这里原来是两个同权重的按钮：「测试连接」只测不存（用输入框里的值），「保存」存了再测。
   * 用户点前者看到「连接成功」，合理地以为配置生效了——它没有。两个动作合并成一个之后，
   * 界面上就不再存在"测试通过但没保存"这种状态。
   */
  async function handleSaveAndTest() {
    if (busy) return
    const normalized = normalize(backendBaseUrl)
    if (!normalized) {
      setResult({ tone: 'warning', message: t('server.urlEmpty') })
      return
    }
    try {
      new URL(normalized)
    } catch {
      setResult({
        tone: 'warning',
        message: t('server.urlInvalid'),
      })
      return
    }

    setBusy(true)
    setResult(null)
    try {
      const next = await persistBackendBaseUrl(normalized)
      setBackendBaseUrl(next)
      setSavedBaseUrl(next)
      setEngineDraftDirty(false)
      // 地址已变更：无论下方健康检查成功与否，都按新地址强制重连，
      // 让左下角连接状态反映新配置（改成错误地址后应显示未连接，而非仍旧"已连接"）
      reconnectProvider()
      // 更新检查跟随这个地址，所以换了地址就要重新查一次（见 getUpdateBaseUrl）。
      // 必须先丢弃已下载的包：ensureDownloaded 只按版本号判断"已经在盘上了"，
      // 版本号相同不代表来自同一台服务器，留着会让"指到测试服务器验一遍"
      // 实际装的还是上一个来源那个包。
      await discardPendingForChannelSwitch()
      void checkForUpdateNow()
    } catch (error) {
      setResult({ tone: 'error', message: t('server.saveFailed'), detail: String(error) })
      setBusy(false)
      return
    }

    try {
      const payload = await probeHealth(normalized)
      setResult(describeHealth(payload, t('server.savedPrefix')))
    } catch (error) {
      const friendly = describeServerError(error, normalize(normalized) !== normalize(defaultBaseUrl))
      setResult({
        tone: 'error',
        message: t('server.savedButUnreachable', { message: friendly.message }),
        detail: friendly.detail,
      })
    } finally {
      setBusy(false)
    }
  }

  /** 恢复到内置默认地址并立刻重连，省得用户自己回忆默认值是什么 */
  async function handleResetDefault() {
    if (busy) return
    setBusy(true)
    setResult(null)
    try {
      const next = await resetBackendBaseUrl()
      setBackendBaseUrl(next)
      setSavedBaseUrl(next)
      setEngineDraftDirty(false)
      reconnectProvider()
      await discardPendingForChannelSwitch()
      void checkForUpdateNow()
      const payload = await probeHealth(next)
      setResult(describeHealth(payload, t('server.restoredPrefix', { url: next })))
    } catch (error) {
      const friendly = describeServerError(error, false)
      setResult({
        tone: 'error',
        message: t('server.restoredButUnreachable', { message: friendly.message }),
        detail: friendly.detail,
      })
    } finally {
      setBusy(false)
    }
  }

  return (
    <>
      <Card>
        <CardContent className="p-6">
          <h2 className="text-lg font-semibold">{t('server.title')}</h2>
          <p className="mt-1 flex items-center gap-1 text-sm text-muted-foreground">
            {t('server.desc')}
            <Tooltip
              variant="light"
              content={t('server.help')}
            >
              <Info className="h-3.5 w-3.5 shrink-0 cursor-help text-muted-foreground transition-colors hover:text-foreground" />
            </Tooltip>
          </p>

          <div className="mt-4">
            <div className="mb-1.5 flex items-center gap-2">
              <label htmlFor="server-base-url" className="text-sm text-muted-foreground">
                {t('server.title')}
              </label>
              {/* 输入框内容只活在 local state 里，切页就没了。原来这件事完全无提示，
                  用户会以为改完就生效了。 */}
              {isDirty && (
                <span className="rounded-full bg-warning/10 px-2 py-0.5 text-xs font-medium text-warning-strong">
                  {t('server.unsaved')}
                </span>
              )}
            </div>
            {/* flex-wrap：800×600 最小窗口下侧栏占掉 192px，输入框 + 按钮挤在一行会溢出 */}
            <div className="flex flex-wrap items-center gap-2">
              <input
                id="server-base-url"
                type="url"
                inputMode="url"
                value={backendBaseUrl}
                onChange={(e) => handleUrlChange(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Enter') void handleSaveAndTest() }}
                placeholder={defaultBaseUrl || 'https://sayitapp.site'}
                className="h-9 min-w-[16rem] flex-1 rounded-md border border-input-border bg-input-bg px-3 text-sm transition-colors focus:border-input-focus-border"
              />
              <Button size="sm" className="h-9 shrink-0" onClick={() => void handleSaveAndTest()} disabled={busy}>
                {busy ? t('server.savingAndTesting') : t('server.saveAndTest')}
              </Button>
              {isCustom && (
                <Button size="sm" variant="ghost" className="h-9 shrink-0" onClick={() => void handleResetDefault()} disabled={busy}>
                  {t('server.restoreDefault')}
                </Button>
              )}
            </div>
          </div>

          {/* 这里原来在失败提示里再挂一个「恢复默认地址（https://…）」按钮：
              一是把整条 URL 塞进按钮文字，全应用没有第二处这么写；
              二是它和输入框旁边那个「恢复默认」完全同义——而后者在地址被改过时一直都在，
              正好覆盖会出现这条失败提示的全部情况。留一个就够。 */}
          {result && (
            <Feedback
              className="mt-3"
              tone={result.tone}
              message={result.message}
              detail={result.detail}
            />
          )}
        </CardContent>
      </Card>

      <Card>
        <CardContent className="p-6">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <div className="min-w-0">
              <h2 id="server-language-heading" className="text-lg font-semibold">{t('server.languageTitle')}</h2>
              <p className="mt-2 text-xs text-muted-foreground">{t('server.languageNote')}</p>
            </div>
            <Segmented
              labelledBy="server-language-heading"
              value={asrLanguage}
              options={[
                { value: 'auto', label: t('common.auto') },
                { value: 'zh', label: t('local.lang.zh') },
                { value: 'en', label: t('local.lang.en') },
              ]}
              onChange={(value) => { setAsrLanguage(value); void setSetting('server.language', value) }}
              className="shrink-0 justify-end"
            />
          </div>
        </CardContent>
      </Card>
    </>
  )
}
