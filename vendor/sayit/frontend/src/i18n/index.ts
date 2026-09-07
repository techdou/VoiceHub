/**
 * 极简 i18n 运行时。
 *
 * 为什么不引 i18next：这里需要的只有「查表 + 插值」，而项目的既有做法是
 * 「纯逻辑抽成文件 + vitest 锁不变量」。自建的最大好处是 **key 类型安全**——
 * 漏 key、拼错 key 在 tsc 阶段就报错，而不是运行时静默 fallback 成 key 本身。
 * locale 文件保持平铺 JSON，将来要迁 i18next 或接 Crowdin 都没有摩擦。
 *
 * ⚠️ 语言只管界面文字，与「识别语种」是两件事：
 * `localAsr.language` / `server.language` 是送给 ASR 的语种，Preset 决定输出语种，
 * 三者都不受这里影响。改这个文件时别顺手把它们串在一起。
 */
import en from './locales/en.json'
import zhCN from './locales/zh-CN.json'

export const LOCALES = ['zh-CN', 'en'] as const
export type Locale = (typeof LOCALES)[number]

/** 语言偏好：'auto' = 跟随系统，解析后一定落在某个具体 Locale 上。 */
export type LanguagePreference = 'auto' | Locale

/** key 从中文表推导 —— 中文表是**源**，新增文案必须先进 zh-CN.json。 */
export type TranslationKey = keyof typeof zhCN

/**
 * en 表显式标注成「与源表 key 完全一致」，少一个 key 就编译不过。
 * 多出来的 key 由 `__tests__/locales.test.ts` 兜住（类型层面查不出多余键）。
 */
const TABLES: Record<Locale, Record<TranslationKey, string>> = {
  'zh-CN': zhCN,
  en,
}

const DEFAULT_LOCALE: Locale = 'zh-CN'

let currentLocale: Locale = DEFAULT_LOCALE
const listeners = new Set<() => void>()

export function isLocale(value: unknown): value is Locale {
  return typeof value === 'string' && (LOCALES as readonly string[]).includes(value)
}

/**
 * 把系统/浏览器给的 locale 串解析成受支持的界面语言。
 *
 * 任何 `zh*`（含 zh-TW / zh-HK）都落到 zh-CN：他们现在看到的就是简体界面，
 * 把他们推到英文是**倒退**。除此之外一律落到英文。
 */
export function resolveLocale(raw: string | null | undefined): Locale {
  const tag = (raw || '').trim().toLowerCase()
  if (!tag) return 'en'
  if (tag.startsWith('zh')) return 'zh-CN'
  return 'en'
}

/** 归一化持久化的偏好值；脏数据一律回落 'auto'。 */
export function normalizePreference(value: unknown): LanguagePreference {
  if (value === 'auto') return 'auto'
  return isLocale(value) ? value : 'auto'
}

export function getLocale(): Locale {
  return currentLocale
}

/**
 * 切换当前界面语言。
 *
 * 同步生效、同步通知，`useT()` 依赖这一点：切语言后当帧就能重渲染，不需要重启。
 * 只写内存，不落库 —— 持久化是 `stores/language.ts` 的职责。
 */
export function setLocale(locale: Locale): void {
  if (!isLocale(locale)) return
  // <html lang> 无条件同步，即使语言没变：初始语言等于默认值时这里是唯一的写入
  // 时机（没有"变化"可言），漏掉就会出现 lang 和实际语言不一致 —— 字体栈跟着
  // lang 走，不一致时没有任何报错，只是英文界面被中文字体渲染。
  applyDocumentLocale(locale)
  if (locale === currentLocale) return
  currentLocale = locale
  listeners.forEach((listener) => listener())
}

/**
 * 把语言写到 <html lang>，字体栈由 index.css 按它切换。
 *
 * 放 CSS 而不是在 JS 里算字体：英文界面下英文字符本该用 Segoe UI 渲染，
 * 中文字体（微软雅黑）渲染英文明显更糊。见 index.css 的 html[lang] 规则。
 */
function applyDocumentLocale(locale: Locale): void {
  if (typeof document === 'undefined') return
  document.documentElement.lang = locale
}

/** 供 useSyncExternalStore 使用。 */
export function subscribeLocale(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

/**
 * 取一条文案。`params` 里的键按 `{name}` 占位替换。
 *
 * 缺 key 时依次回落：当前语言 → 中文源表 → key 本身。回落到 key 本身意味着
 * 界面上会出现 `nav.home` 这种东西，刺眼是故意的 —— 它比显示空字符串好排查。
 */
export function t(key: TranslationKey, params?: Record<string, string | number>): string {
  const table = TABLES[currentLocale]
  const template = table[key] ?? TABLES[DEFAULT_LOCALE][key] ?? key
  if (!params) return template
  return template.replace(/\{(\w+)\}/g, (match, name: string) => {
    const value = params[name]
    return value === undefined ? match : String(value)
  })
}
