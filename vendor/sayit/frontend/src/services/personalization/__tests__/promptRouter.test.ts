import { describe, it, expect } from 'vitest'
import { matchesAppPromptRule } from '../promptRouter'
import { BUILTIN_APP_RULES } from '../defaults'
import type { AppPromptRule } from '../types'
import type { ActiveAppContext } from '@/types/appContext'

function ruleById(id: string): AppPromptRule {
  const rule = BUILTIN_APP_RULES.find((r) => r.id === id)
  if (!rule) throw new Error(`内置规则不存在: ${id}`)
  return rule
}

describe('matchesAppPromptRule', () => {
  it('进程名匹配时命中', () => {
    const ctx: ActiveAppContext = { processName: 'outlook.exe', windowTitle: '收件箱 - Outlook' }
    expect(matchesAppPromptRule(ruleById('outlook'), ctx)).toBe(true)
  })

  it('拿到进程名时，标题不再独立触发：在 Outlook 里写含 Teams 的邮件不应命中 Teams', () => {
    // 这是改动前的真实误判：标题「包含」teams 就命中 Teams 规则，
    // 且 Teams 优先级(100) > Outlook(95)，正式邮件会被按即时聊天风格整理。
    const ctx: ActiveAppContext = { processName: 'outlook.exe', windowTitle: 'Teams 会议纪要 - Outlook' }
    expect(matchesAppPromptRule(ruleById('teams'), ctx)).toBe(false)
    expect(matchesAppPromptRule(ruleById('outlook'), ctx)).toBe(true)
  })

  it('短关键词不再误伤：在 VSCode 里编辑 qq_faq.md 不应命中 QQ', () => {
    const ctx: ActiveAppContext = { processName: 'code.exe', windowTitle: 'qq_faq.md - proj - Visual Studio Code' }
    expect(matchesAppPromptRule(ruleById('qq'), ctx)).toBe(false)
    expect(matchesAppPromptRule(ruleById('vscode'), ctx)).toBe(true)
  })

  it('进程名对不上就是不命中（不再靠标题捞回来）', () => {
    const ctx: ActiveAppContext = { processName: 'chrome.exe', windowTitle: 'Outlook - Google Chrome' }
    expect(matchesAppPromptRule(ruleById('outlook'), ctx)).toBe(false)
  })

  it('规则没写进程名时（如网页版应用）回落到标题匹配', () => {
    const webRule: AppPromptRule = {
      ...ruleById('outlook'),
      id: 'outlook-web',
      matcher: { processNames: [], windowTitleIncludes: ['outlook'] },
    }
    const ctx: ActiveAppContext = { processName: 'msedge.exe', windowTitle: '收件箱 - Outlook - Microsoft Edge' }
    expect(matchesAppPromptRule(webRule, ctx)).toBe(true)
  })

  it('拿不到进程名时仍回落到标题匹配（不能因为进程名为空就判不命中）', () => {
    // 内置规则已不带标题列表，这里显式构造一条"两者都写"的规则来验证兜底分支
    const rule: AppPromptRule = {
      ...ruleById('outlook'),
      matcher: { processNames: ['outlook.exe'], windowTitleIncludes: ['outlook'] },
    }
    const ctx: ActiveAppContext = { windowTitle: '收件箱 - Outlook' }
    expect(matchesAppPromptRule(rule, ctx)).toBe(true)
  })

  it('内置规则不再带窗口标题列表（避免标题匹配把别的应用抢走）', () => {
    for (const rule of BUILTIN_APP_RULES) {
      expect(rule.matcher.processNames.length).toBeGreaterThan(0)
      expect(rule.matcher.windowTitleIncludes ?? []).toHaveLength(0)
    }
  })

  it('进程名从 exePath 兜底解析', () => {
    const ctx: ActiveAppContext = { exePath: 'C:\\Program Files\\Notepad\\notepad.exe' }
    expect(matchesAppPromptRule(ruleById('notepad'), ctx)).toBe(true)
  })

  it('无上下文时不命中', () => {
    expect(matchesAppPromptRule(ruleById('teams'), null)).toBe(false)
  })
})
