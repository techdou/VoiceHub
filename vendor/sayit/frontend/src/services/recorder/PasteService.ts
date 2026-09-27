import * as bridge from '../bridge'
import type { TextInsertionAttempt, TextInsertionResult } from '../textInsertion'

export interface ProbeResult {
  editable: boolean
  hwnd: string
  process: string
  detail: string
  pid?: number
  tid?: number
  focusHwnd?: string
  caretHwnd?: string
  caret?: number
  hasCaret?: boolean
  control?: number
  verdict?: string
  /** 结论是四层判据里哪一层给出的（caret / native_class / uia_* / chromium_optimistic
   *  / process_allowlist / no_signal）。取值定义在 inject/mod.rs 的 EditableGate。 */
  gate?: string
  probeId?: number
  startedAt?: number
  completedAt?: number
  isCurrentAppProcess?: boolean
  windowClass?: string
  focusClass?: string
  controlType?: string
  automationId?: string
  isValuePatternAvailable?: boolean
  isKeyboardFocusable?: boolean
  isEnabled?: boolean
  isReadOnly?: boolean
}

export interface PasteResult extends TextInsertionResult {
  attempts?: TextInsertionAttempt[]
}

export class PasteService {
  /**
   * Get the pre-probed editable result (probed at PTT down in main process).
   */
  async getProbeResult(): Promise<ProbeResult> {
    try {
      const result = await bridge.getProbeResult()
      if (!result) return { editable: false, hwnd: '0', process: '-', detail: 'no_api' }
      return result as unknown as ProbeResult
    } catch {
      return { editable: false, hwnd: '0', process: '-', detail: 'probe_error' }
    }
  }

  /**
   * Paste text using pre-probed hwnd/focusHwnd.
   * Passing the hwnd avoids re-capturing context after PTT release
   * (which may return the wrong foreground window).
   */
  async pasteText(text: string, probe?: ProbeResult, restoreClipboard?: boolean): Promise<PasteResult> {
    try {
      const result = await bridge.pasteText(
        text,
        probe?.hwnd,
        probe?.focusHwnd,
        restoreClipboard,
      )
      if (!result) return { ok: false, reason: 'no_result' }
      return result as PasteResult
    } catch (error) {
      return {
        ok: false,
        reason: 'paste_exception',
        detail: String(error),
      }
    }
  }
}
