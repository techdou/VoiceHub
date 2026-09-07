import type { ActiveAppContext } from '@/types/appContext'
import type { PromptPreset } from '@/services/store'

export interface AppPromptMatcher {
  processNames: string[]
  windowTitleIncludes?: string[]
  windowClasses?: string[]
  automationIds?: string[]
}

/**
 * 一条应用规则。
 *
 * **顺序即优先级**：数组下标越小越先命中（见 promptRouter.resolvePromptRouting），
 * 界面上就是自上而下。以前这里有个 `priority` 数字，界面按它排序 —— 用户看得见顺序、
 * 却改不了顺序；现在顺序由用户拖动决定，就不能再留一个会跟顺序打架的字段。
 * 旧存档里的 `priority` 读取时忽略（当时存下来的数组本身已经是按它排好序的）。
 */
export interface AppPromptRule {
  id: string
  appId: string
  name: string
  enabled: boolean
  builtin?: boolean
  presetId?: string
  promptAppend: string
  matcher: AppPromptMatcher
}

export interface UserStats {
  totalWords: number
  totalSessions: number
  domainWords: Record<string, number>
  appUsageCount: Record<string, number>
  firstUsedAt?: number
  lastUsedAt?: number
}

export interface PromptResolution {
  appId?: string
  appName?: string
  preset: PromptPreset
  matchedRule?: AppPromptRule
  systemPrompt: string
  summary: string
}

export interface PromptRoutingInput {
  appContext: ActiveAppContext | null
  presets: PromptPreset[]
  activePresetId: string
  appRules: AppPromptRule[]
  userStats: UserStats
  /** 生效的热词列表（用于可选的"注入 AI 提示词"功能） */
  hotwords?: string[]
  /** 是否把热词注入到 AI 系统提示词，帮助 AI 纠正专有名词。默认关闭。 */
  injectHotwords?: boolean
}
