import type { AppPromptRule, UserStats } from './types'

// 这里的**数组顺序就是默认优先级**（自上而下先命中先生效，见 promptRouter）。
// 用户可以在界面上拖动调整，调整后的顺序存进 appPromptRules；这份清单只决定
// 「从没动过」时的初始顺序，以及后续版本新增的内置规则追加在哪。
//
// 内置规则一律只靠 processNames 判定（见 promptRouter.matchesAppPromptRule）。
// 这里**不要**再加 windowTitleIncludes：标题是「包含即命中」，一个标题能同时命中
// 多条规则 —— 曾导致在 Outlook 里写标题含「Teams」的邮件被 Teams 规则抢走。
// 标题匹配只保留给「进程名区分不了」的场景（如网页版应用，进程都是浏览器）。

export const BUILTIN_APP_RULES: AppPromptRule[] = [
  {
    id: 'teams',
    appId: 'teams',
    name: 'Teams',
    builtin: true,
    enabled: false,
    presetId: 'intent',
    promptAppend: '适合即时协作聊天。优先输出可以直接发送的短消息，语气自然、清晰、简洁，避免邮件腔。', // i18n-allow: 中文口述整理 Prompt
    matcher: {
      processNames: ['teams.exe', 'ms-teams.exe'],
    },
  },
  {
    id: 'outlook',
    appId: 'outlook',
    name: 'Outlook',
    builtin: true,
    enabled: false,
    presetId: 'intent',
    promptAppend: '适合工作邮件草稿。语气正式、完整，必要时自然分段，但不要编造收件人、称呼或任何事实。', // i18n-allow: 中文口述整理 Prompt
    matcher: {
      processNames: ['outlook.exe', 'olk.exe'],
    },
  },
  {
    id: 'kiro',
    appId: 'kiro',
    name: 'Kiro',
    builtin: true,
    enabled: false,
    presetId: 'faithful',
    promptAppend: '面向开发工具输入。保留代码、命令、文件名、路径、英文标识符和 Markdown 结构，不要把技术词改写成普通中文。', // i18n-allow: 中文口述整理 Prompt
    matcher: {
      processNames: ['kiro.exe'],
    },
  },
  {
    id: 'codex',
    appId: 'codex',
    name: 'Codex',
    builtin: true,
    enabled: false,
    presetId: 'faithful',
    promptAppend: '面向 Codex 编码工具，多为用自然语言下达编程指令。保留代码、命令、文件名、路径和英文标识符，把要做的事说清楚，不要把技术词改写成普通中文。', // i18n-allow: 中文口述整理 Prompt
    matcher: {
      processNames: ['codex.exe'],
    },
  },
  {
    id: 'vscode',
    appId: 'vscode',
    name: 'VSCode',
    builtin: true,
    enabled: false,
    presetId: 'faithful',
    promptAppend: '面向 VSCode 编辑区。保留代码、命令、文件名、API、英文术语和 Markdown 结构，不要过度润色技术内容。', // i18n-allow: 中文口述整理 Prompt
    matcher: {
      processNames: ['code.exe'],
    },
  },
  {
    id: 'cursor',
    appId: 'cursor',
    name: 'Cursor',
    builtin: true,
    enabled: false,
    presetId: 'faithful',
    promptAppend: '面向 Cursor 编辑区。保留代码、命令、路径、技术名词和英文标识符，不要擅自解释或补全技术内容。', // i18n-allow: 中文口述整理 Prompt
    matcher: {
      processNames: ['cursor.exe'],
    },
  },
  {
    id: 'notepad',
    appId: 'notepad',
    name: 'Notepad',
    builtin: true,
    enabled: false,
    presetId: 'intent',
    promptAppend: '面向 Windows 记事本，适合随手记录的纯文本。输出纯文本，不要使用 Markdown 标记或特殊格式符号。', // i18n-allow: 中文口述整理 Prompt
    matcher: {
      processNames: ['notepad.exe'],
    },
  },
  {
    id: 'weixin',
    appId: 'weixin',
    name: 'WeChat',
    builtin: true,
    enabled: false,
    presetId: 'casual',
    promptAppend: '适合微信聊天。输出可直接发送的自然口语短消息，简洁亲切，不要用书面或邮件腔。', // i18n-allow: 中文口述整理 Prompt
    matcher: {
      processNames: ['weixin.exe', 'wechat.exe'],
    },
  },
  {
    id: 'qq',
    appId: 'qq',
    name: 'QQ',
    builtin: true,
    enabled: false,
    presetId: 'casual',
    promptAppend: '适合 QQ 聊天。输出轻松自然、可直接发送的短消息，口语化、简洁。', // i18n-allow: 中文口述整理 Prompt
    matcher: {
      processNames: ['qq.exe'],
    },
  },
]

export function createDefaultUserStats(): UserStats {
  return {
    totalWords: 0,
    totalSessions: 0,
    domainWords: {},
    appUsageCount: {},
  }
}
