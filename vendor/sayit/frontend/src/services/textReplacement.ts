// 文本替换服务 — 对 ASR 识别结果做客户端侧的文本替换

import { getSetting, setSetting } from './store'

export interface TextReplacementRule {
  id: string
  from: string
  to: string
  enabled: boolean
}

const STORAGE_KEY = 'textReplacements'

/** 内置默认文本替换规则（新用户首次使用时填充） */
// i18n-allow-start: 中文 ASR 默认替换数据，不是界面文案
export const BUILTIN_REPLACEMENTS: TextReplacementRule[] = [
  { id: 'builtin_1', from: '安卓说话', to: '按住说话', enabled: true },
  { id: 'builtin_2', from: '我的邮箱', to: 'test@example.com', enabled: true },
  { id: 'builtin_3', from: 'pump', to: 'Prompt', enabled: true },
  { id: 'builtin_4', from: 'Cloud Code', to: 'Claude Code', enabled: true },
]
// i18n-allow-end

export async function getTextReplacements(): Promise<TextReplacementRule[]> {
  const rules = await getSetting<TextReplacementRule[]>(STORAGE_KEY, [])
  // 新用户没有任何规则时，返回内置默认
  if (!rules || rules.length === 0) {
    return BUILTIN_REPLACEMENTS.map((r) => ({ ...r }))
  }
  return rules
}

export async function saveTextReplacements(rules: TextReplacementRule[]): Promise<void> {
  await setSetting(STORAGE_KEY, rules)
}

/** 批量文本解析出的规则草稿（尚未分配 id/enabled） */
export interface ParsedReplacement {
  from: string
  to: string
}

/**
 * 解析批量输入文本为替换规则草稿。
 * - 每行一条规则；空行忽略。
 * - 分隔符支持：制表符（从表格粘贴）、`=>`、`->`、中文逗号「，」、英文逗号「,」。
 *   取整行中最靠前出现的分隔符切分：分隔符前为「原文」，其后（含其余分隔符）为「替换为」。
 * - 无分隔符的行：整行作为「原文」，替换为留空（即删除该文本）。
 * - 原文为空的行忽略。
 */
export function parseBatchReplacements(input: string): ParsedReplacement[] {
  const separators = ['\t', '=>', '->', '，', ','] // i18n-allow: 中文批量输入分隔符
  const result: ParsedReplacement[] = []

  for (const rawLine of input.split(/\r?\n/)) {
    const line = rawLine.trim()
    if (!line) continue

    let sepIndex = -1
    let sepLen = 0
    for (const sep of separators) {
      const i = line.indexOf(sep)
      if (i >= 0 && (sepIndex === -1 || i < sepIndex)) {
        sepIndex = i
        sepLen = sep.length
      }
    }

    let from = line
    let to = ''
    if (sepIndex >= 0) {
      from = line.slice(0, sepIndex).trim()
      to = line.slice(sepIndex + sepLen).trim()
    }

    if (!from) continue
    result.push({ from, to })
  }

  return result
}

/** 对文本应用所有启用的替换规则 */
export function applyReplacements(text: string, rules: TextReplacementRule[]): string {
  let result = text
  for (const rule of rules) {
    if (rule.enabled && rule.from) {
      result = result.split(rule.from).join(rule.to)
    }
  }
  return result
}

/** 加载规则并应用替换（便捷方法） */
export async function applyTextReplacements(text: string): Promise<string> {
  if (!text) return text
  const rules = await getTextReplacements()
  if (rules.length === 0) return text
  return applyReplacements(text, rules)
}
