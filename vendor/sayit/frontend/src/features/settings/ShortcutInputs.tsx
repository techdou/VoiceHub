import * as bridge from '@/services/bridge'
import { setShortcutCaptureActive } from '@/services/webviewKeyboardFallback'
import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react'
import { X } from 'lucide-react'
import { t } from '@/i18n'
import {
  displayAccelerator,
  eventToAccelerator,
  getSingleKeyDisplay,
  resolveSingleKeyShortcut,
} from './utils'
import {
  canonicalizePTTShortcut,
  displayPTTShortcut,
  getAcceleratorShortcutValidationError,
  getPTTShortcutWarning,
  getPTTShortcutValidationError,
} from '@/lib/shortcutKeys'

/** 提交前校验：返回错误文案则拒绝保存（用于应用内部快捷键互斥等），返回 null 放行。 */
export type ShortcutValidate = (value: string) => Promise<string | null>

/** 冲突提示的存活时间：读完就该消失。 */
const ERROR_VISIBLE_MS = 6000

/**
 * 一次性的错误提示：显示一段时间后自动消失。
 *
 * 这类提示只说明"刚才那次按键为什么没被采纳"，一旦用户改动了冲突的另一个键（比如把
 * 「按住说话」换成别的），它就成了没人再管的陈述——挂在页面上不走，看着像设置一直有错。
 */
function useTransientMessage() {
  const [message, setMessage] = useState('')
  const timerRef = useRef<number | undefined>(undefined)
  const show = useCallback((next: string) => {
    window.clearTimeout(timerRef.current)
    setMessage(next)
    if (next) timerRef.current = window.setTimeout(() => setMessage(''), ERROR_VISIBLE_MS)
  }, [])
  useEffect(() => () => window.clearTimeout(timerRef.current), [])
  return [message, show] as const
}

let suspendCount = 0
let activeCaptureCancel: (() => void) | null = null

function isShortcutCaptureOwner(cancel: () => void) {
  return activeCaptureCancel === cancel
}

/**
 * 录制期间挂起本应用自己的全部热键，结束后恢复。
 *
 * 不挂起的话，按到已绑定的键（如免提的右 Alt）会先被热键链路吃掉：webview 回退监听
 * 直接 emit toggle-hands-free 弹出悬浮窗开始录音，组合键则被 RegisterHotKey 抢走、
 * 根本送不到 webview——两种情况都录不进新设置。
 *
 * `cancel` 必须是稳定引用（useCallback），否则每次渲染都会重挂一遍挂起/恢复。
 */
function useSuspendHotkeys(active: boolean, cancel: () => void) {
  useEffect(() => {
    if (!active) return
    // 全页面只允许一个录制 owner。打开新框时先取消旧框，避免两个 window 监听器
    // 同时保存同一次按键，并绕过双方基于旧设置快照做的互斥校验。
    if (activeCaptureCancel && activeCaptureCancel !== cancel) {
      activeCaptureCancel()
    }
    activeCaptureCancel = cancel

    // 计数保护 effect 清理的短暂交叠：旧 owner 的 cleanup 不能提前恢复热键。
    suspendCount += 1
    if (suspendCount === 1) {
      setShortcutCaptureActive(true)
      bridge.beginShortcutCapture()
    }
    // 切走到别的程序就取消录制：录制态一直挂着的话，热键会一直处于挂起状态，
    // 用户回头只会觉得"热键突然不管用了"。
    window.addEventListener('blur', cancel)
    return () => {
      window.removeEventListener('blur', cancel)
      if (activeCaptureCancel === cancel) activeCaptureCancel = null
      suspendCount -= 1
      if (suspendCount === 0) {
        setShortcutCaptureActive(false)
        bridge.endShortcutCapture()
      }
    }
  }, [active, cancel])
}

export function PTTShortcutInput({
  value,
  onChange,
  label,
  description,
  validate,
}: {
  value: string
  onChange: (value: string) => void
  label: ReactNode
  description: string
  validate?: ShortcutValidate
}) {
  const [recording, setRecording] = useState(false)
  const [tempValue, setTempValue] = useState('')
  const [validateError, showValidateError] = useTransientMessage()
  // 仅在“本次刚绑定中键”后提示一次；重进页面（组件重挂载）不再显示。
  const [showMiddleHint, setShowMiddleHint] = useState(false)
  const pressedRef = useRef(new Set<string>())
  const peakRef = useRef(new Set<string>())
  const committingRef = useRef(false)

  const resetCapture = useCallback(() => {
    pressedRef.current.clear()
    peakRef.current.clear()
    committingRef.current = false
    setTempValue('')
  }, [])

  const cancelRecording = useCallback(() => {
    setRecording(false)
    resetCapture()
  }, [resetCapture])
  useSuspendHotkeys(recording, cancelRecording)

  const commit = useCallback(async (mapped: string) => {
    const canonical = canonicalizePTTShortcut(mapped)
    const formatError = getPTTShortcutValidationError(canonical)
    if (formatError) {
      showValidateError(formatError)
      return false
    }
    const error = validate ? await validate(canonical) : null
    if (error) {
      showValidateError(error)
      return false
    }
    showValidateError('')
    onChange(canonical)
    return true
  }, [onChange, validate, showValidateError])

  const handleKeyDown = useCallback((event: KeyboardEvent) => {
    if (!isShortcutCaptureOwner(cancelRecording)) return
    event.preventDefault()
    event.stopPropagation()
    if (event.repeat || committingRef.current || pressedRef.current.has(event.code)) return

    pressedRef.current.add(event.code)
    peakRef.current.add(event.code)
    setTempValue(canonicalizePTTShortcut(peakRef.current))
  }, [cancelRecording])

  const handleKeyUp = useCallback((event: KeyboardEvent) => {
    if (!isShortcutCaptureOwner(cancelRecording)) return
    event.preventDefault()
    event.stopPropagation()
    if (!pressedRef.current.delete(event.code) || committingRef.current) return

    // 第一次松开时保存本次同时按下过的峰值；之后的 keyup 不会重复提交。
    committingRef.current = true
    const candidate = canonicalizePTTShortcut(peakRef.current)
    setRecording(false)
    void commit(candidate).finally(resetCapture)
  }, [cancelRecording, commit, resetCapture])

  useEffect(() => {
    if (!recording) return
    window.addEventListener('keydown', handleKeyDown)
    window.addEventListener('keyup', handleKeyUp)
    // 鼠标侧键无法靠 webview 事件可靠捕获（会被当成“后退”导航），改由 Rust 底层鼠标钩子
    // 在 OS 层捕获并吞掉，再通过事件回报要绑定的侧键（捕获模式由 useSuspendHotkeys 开启）。
    const off = bridge.onMouseShortcutCaptured(({ setting }) => {
      if (!isShortcutCaptureOwner(cancelRecording)) return
      if (!setting || committingRef.current) return
      committingRef.current = true
      setRecording(false)
      pressedRef.current.clear()
      peakRef.current.clear()
      setTempValue('')
      // 侧键：延迟提交（延迟触发钩子重配），让本次物理“松开”先被当前钩子吞掉。
      // 否则重配钩子的空档期会把这次“抬起”漏给 webview——后退键会导致页面返回。
      window.setTimeout(() => {
        void commit(setting).then((ok) => setShowMiddleHint(ok && setting === 'MButton'))
          .finally(() => { committingRef.current = false })
      }, 400)
    })
    return () => {
      window.removeEventListener('keydown', handleKeyDown)
      window.removeEventListener('keyup', handleKeyUp)
      off()
    }
  }, [recording, handleKeyDown, handleKeyUp, cancelRecording, commit])

  const displayValue = tempValue || value
  const keys = displayValue ? displayPTTShortcut(displayValue) : [t('shortcut.notSet')]
  const shortcutWarning = !recording ? getPTTShortcutWarning(value) : null

  return (
    <div>
      <div className="flex items-center justify-between gap-4">
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium">{label}</p>
          <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">{description}</p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <button
            onClick={() => {
              showValidateError('')
              if (recording) {
                cancelRecording()
              } else {
                resetCapture()
                setRecording(true)
              }
            }}
            className={`flex items-center justify-center gap-1 whitespace-nowrap rounded-md border px-2 py-1.5 text-sm transition-colors ${recording
              ? 'border-primary bg-primary/5 ring-2 ring-primary/20'
              : 'border-input bg-muted hover:bg-accent'
              }`}
          >
            {recording && !tempValue ? (
              <span className="animate-pulse text-muted-foreground">{t('shortcutInput.pressKeys')}</span>
            ) : (
              keys.map((key, index) => (
                <span key={`${key}-${index}`}>
                  {index > 0 && <span className="mx-0.5 text-muted-foreground">+</span>}
                  <span className={`rounded border bg-card px-1.5 py-0.5 text-xs shadow-sm ${!displayValue ? 'text-muted-foreground' : ''}`}>
                    {key}
                  </span>
                </span>
              ))
            )}
          </button>

          {!recording && value && (
            <button
              onClick={() => onChange('')}
              className="rounded p-1 hover:bg-accent"
              title={t('shortcutInput.clear')}
              aria-label={t('shortcutInput.clear')}
            >
              <X className="h-3.5 w-3.5 text-muted-foreground" />
            </button>
          )}
        </div>
      </div>
      {validateError && (
        <p className="mt-1.5 text-xs text-destructive">{validateError}</p>
      )}
      {shortcutWarning && (
        <p className="mt-1.5 text-xs text-amber-500">{shortcutWarning}</p>
      )}
      {showMiddleHint && value === 'MButton' && (
        <p className="mt-1.5 text-xs text-amber-500">
          {t('shortcutInput.middleMouseWarning')}
        </p>
      )}
    </div>
  )
}

export function ComboShortcutInput({
  value,
  onChange,
  label,
  description,
  comboOnly = false,
  allowMouseShortcut = false,
  validate,
}: {
  value: string
  onChange: (value: string) => void
  label: ReactNode
  description: string
  /** 仅接受"修饰键+主键"的组合键，拒绝单键/单修饰键（用于预设切换快捷键）。 */
  comboOnly?: boolean
  /** 组合键为主的设置也可额外允许鼠标侧键/中键（AI 整理开关）。 */
  allowMouseShortcut?: boolean
  validate?: ShortcutValidate
}) {
  const [recording, setRecording] = useState(false)
  const [tempValue, setTempValue] = useState('')
  const [conflict, showConflict] = useTransientMessage()
  // 仅在"本次刚绑定中键"后提示一次；重进页面（组件重挂载）不再显示。
  const [showMiddleHint, setShowMiddleHint] = useState(false)
  // 一次录制只提交/探测一次：松开组合键会产生多个 keyup，
  // 若不加守卫会对同一组合键重复调用 test_shortcut，第二次因“自己刚注册”而误报冲突。
  const committingRef = useRef(false)

  const cancelRecording = useCallback(() => { setRecording(false); setTempValue('') }, [])
  useSuspendHotkeys(recording, cancelRecording)

  const handleKeyDown = useCallback((event: KeyboardEvent) => {
    if (!isShortcutCaptureOwner(cancelRecording)) return
    event.preventDefault()
    event.stopPropagation()

    // 非 comboOnly：优先接受单键（如免提用右 Alt）
    if (!comboOnly) {
      const singleKey = resolveSingleKeyShortcut(event.code)
      if (singleKey) {
        setTempValue(singleKey)
        return
      }
    }

    // 组合键：仅当"修饰键 + 主键"时 eventToAccelerator 才返回值；
    // 单独按修饰键（Ctrl/Alt/Shift）返回 null，不会被误提交。
    const accelerator = eventToAccelerator(event)
    if (accelerator) setTempValue(accelerator)
  }, [cancelRecording, comboOnly])

  const handleKeyUp = useCallback((event: KeyboardEvent) => {
    if (!isShortcutCaptureOwner(cancelRecording)) return
    event.preventDefault()
    event.stopPropagation()
    if (!tempValue || committingRef.current) return

    // 如果是单键：comboOnly 模式拒绝，非 comboOnly 校验后保存
    const isSingle = resolveSingleKeyShortcut(tempValue) !== undefined
    if (isSingle) {
      if (comboOnly) return
      committingRef.current = true
      void (async () => {
        const systemError = getAcceleratorShortcutValidationError(tempValue)
        const error = systemError || (validate ? await validate(tempValue) : null)
        if (error) {
          showConflict(error)
        } else {
          showConflict('')
          onChange(tempValue)
        }
        setRecording(false)
        setTempValue('')
      })()
      return
    }

    // 组合键：先查应用内部冲突，再探测是否被其他程序占用。
    // 加守卫避免多次 keyup 重复探测。
    committingRef.current = true
    void (async () => {
      const systemError = getAcceleratorShortcutValidationError(tempValue)
      const error = systemError || (validate ? await validate(tempValue) : null)
      if (error) {
        showConflict(error)
      } else if (await bridge.testShortcut(tempValue)) {
        showConflict('')
        onChange(tempValue)
      } else {
        showConflict(t('shortcutInput.conflictOther'))
      }
      setRecording(false)
      setTempValue('')
    })()
  }, [cancelRecording, tempValue, onChange, comboOnly, validate, showConflict])

  useEffect(() => {
    if (!recording) return

    window.addEventListener('keydown', handleKeyDown)
    window.addEventListener('keyup', handleKeyUp)
    // 鼠标侧键由 Rust 底层鼠标钩子捕获后回报。少数以组合键为主的设置（AI 整理开关）
    // 也可显式放行鼠标单键。
    // （捕获模式由 useSuspendHotkeys 开启）。
    let off: (() => void) | undefined
    if (!comboOnly || allowMouseShortcut) {
      off = bridge.onMouseShortcutCaptured(({ setting }) => {
        if (!isShortcutCaptureOwner(cancelRecording)) return
        if (!setting || committingRef.current) return
        committingRef.current = true
        setRecording(false)
        setTempValue('')
        // 见 PTT 处说明：延迟提交，避免重配钩子的空档期把侧键“抬起”漏给 webview。
        window.setTimeout(() => {
          void (async () => {
            const systemError = getAcceleratorShortcutValidationError(setting)
            const error = systemError || (validate ? await validate(setting) : null)
            if (error) {
              showConflict(error)
              return
            }
            showConflict('')
            setShowMiddleHint(setting === 'MButton')
            onChange(setting)
          })()
        }, 400)
      })
    }
    return () => {
      window.removeEventListener('keydown', handleKeyDown)
      window.removeEventListener('keyup', handleKeyUp)
      off?.()
    }
  }, [recording, handleKeyDown, handleKeyUp, comboOnly, allowMouseShortcut, cancelRecording, onChange, validate, showConflict])

  // 显示：单键用 getSingleKeyDisplay，组合键用 displayAccelerator
  const isSingleKey = resolveSingleKeyShortcut(tempValue || value) !== undefined
  const displayValue = tempValue || value || ''
  const keys = !displayValue
    ? [t('shortcut.notSet')]
    : isSingleKey
      ? [getSingleKeyDisplay(displayValue)]
      : displayAccelerator(displayValue)

  return (
    <div>
      <div className="flex items-center justify-between gap-4">
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium">{label}</p>
          <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">{description}</p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <button
            onClick={() => {
              committingRef.current = false
              setRecording(!recording)
              setTempValue('')
              showConflict('')
            }}
            className={`flex items-center justify-center gap-1 whitespace-nowrap rounded-md border px-2 py-1.5 text-sm transition-colors ${recording
              ? 'border-primary bg-primary/5 ring-2 ring-primary/20'
              : 'border-input bg-muted hover:bg-accent'
              }`}
          >
            {recording && !tempValue ? (
              <span className="animate-pulse text-muted-foreground">{t('shortcutInput.pressKeys')}</span>
            ) : (
              keys.map((key, index) => (
                <span key={index}>
                  {index > 0 && <span className="mx-0.5 text-muted-foreground">+</span>}
                  <span className="rounded border bg-card px-1.5 py-0.5 text-xs shadow-sm">{key}</span>
                </span>
              ))
            )}
          </button>

          {!recording && (
            <button
              onClick={() => onChange('')}
              className="rounded p-1 hover:bg-accent"
              aria-label={t('shortcutInput.clear')}
            >
              <X className="h-3.5 w-3.5 text-muted-foreground" />
            </button>
          )}
        </div>
      </div>
      {conflict && (
        <p className="mt-1.5 text-xs text-destructive">{conflict}</p>
      )}
      {showMiddleHint && value === 'MButton' && (
        <p className="mt-1.5 text-xs text-amber-500">{t('shortcutInput.middleMouseWarning')}</p>
      )}
    </div>
  )
}
