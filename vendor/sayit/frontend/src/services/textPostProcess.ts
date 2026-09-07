// 文本后处理 — 不依赖 AI 的客户端文本规范化
//
// 提供三类可开关的处理：
//   1. 数字规范化（A 档，尽力而为）：百分之 / 小数点 / 分之 / 含位值词的结构化整数
//   2. 去除句末标点
//   3. 标点符号替换为空格
//
// 处理顺序在 applyTextTransforms 中固定：数字规范化 → 用户替换规则 → 去句末标点 → 标点转空格。
// 数字规范化必须在标点处理之前（小数点依赖「点」，标点转空格会吃掉标点）。

import { getSetting, setSetting } from './store'
import { applyTextReplacements } from './textReplacement'
import { segmentAsrText } from './textSegmenter'

// ── 中文数字字符表 ──
// i18n-allow-start: 中文数字解析算法的输入与输出标记
const CN_DIGITS: Record<string, number> = {
  '零': 0, '〇': 0, '一': 1, '二': 2, '两': 2, '三': 3, '四': 4,
  '五': 5, '六': 6, '七': 7, '八': 8, '九': 9, '幺': 1,
}
const CN_SMALL_UNITS: Record<string, number> = { '十': 10, '百': 100, '千': 1000 }
const CN_BIG_UNITS: Record<string, number> = { '万': 10000, '亿': 100000000 }

/** 所有中文数字字符（含单位） */
const NUM_CHARS = '零〇一二两三四五六七八九十百千万亿幺'
/** 纯数字字符（不含单位），用于小数部分逐位读取 */
const DIGIT_CHARS = '零〇一二两三四五六七八九幺'
/** 位值单位字符 */
const UNIT_CHARS = '十百千万亿'

/**
 * 连续单位词构成的固定词/成语黑名单：这些通过了「长度≥2 且含单位」的过滤，
 * 但语义上不是数字，需排除。多数含单位的成语（千方百计、十全十美等）因为
 * 单位与非数字字符交替出现、拆分后单位单独成段（长度<2）已被自动跳过，
 * 这里只列举「多个单位/数字连续」的高频误伤词。可按需扩充。
 */
const INTEGER_BLACKLIST = new Set([
  '千万', '万一', '万万', '千千万万', '亿万', '万千', '十万八千',
])

/** 解析一段中文整数为数值；无法解析返回 null。支持口语省略（三千二=3200、一万五=15000）。 */
function parseChineseInteger(s: string): number | null {
  if (!s) return null
  let total = 0
  let section = 0
  let number = 0
  let lastUnit = 0 // 最近一次使用的单位量级（大小单位皆计）
  let sawZero = false
  let hadAny = false

  for (const ch of s) {
    if (ch in CN_DIGITS) {
      number = CN_DIGITS[ch]
      if (ch === '零' || ch === '〇') sawZero = true
      hadAny = true
    } else if (ch in CN_SMALL_UNITS) {
      const unit = CN_SMALL_UNITS[ch]
      if (number === 0) number = 1 // 「十五」→ 十 隐含 1
      section += number * unit
      number = 0
      lastUnit = unit
      hadAny = true
    } else if (ch in CN_BIG_UNITS) {
      const unit = CN_BIG_UNITS[ch]
      section += number
      total += section * unit
      section = 0
      number = 0
      lastUnit = unit
      sawZero = false
      hadAny = true
    } else {
      return null
    }
  }

  if (!hadAny) return null
  // 口语省略的末位数字：无「零」时按下一量级缩放（三千二=3200、一万五=15000、两百五=250）
  if (number !== 0 && lastUnit >= 10 && !sawZero) {
    section += number * (lastUnit / 10)
    number = 0
  }
  return total + section + number
}

/** 解析可能带小数点的中文数字表达为字符串；失败返回 null。 */
function parseNumberExpr(s: string): string | null {
  const dotIdx = s.indexOf('点')
  if (dotIdx === -1) {
    const n = parseChineseInteger(s)
    return n === null ? null : String(n)
  }
  const intPart = s.slice(0, dotIdx)
  const decPart = s.slice(dotIdx + 1)
  if (!decPart) return null
  const intVal = intPart ? parseChineseInteger(intPart) : 0
  if (intVal === null) return null
  let dec = ''
  for (const ch of decPart) {
    if (ch in CN_DIGITS) dec += String(CN_DIGITS[ch])
    else return null
  }
  return `${intVal}.${dec}`
}

function containsUnit(s: string): boolean {
  for (const ch of s) if (UNIT_CHARS.includes(ch)) return true
  return false
}

/** 若替换处紧跟在英文字母之后，前面补一个空格（如 GPT五点四 → GPT 5.4）。 */
function maybeSpace(str: string, offset: number, replacement: string): string {
  const prev = offset > 0 ? str[offset - 1] : ''
  return /[A-Za-z]/.test(prev) ? ' ' + replacement : replacement
}

/**
 * 中文数字 → 阿拉伯数字（A 档：有明确信号才转，尽力而为）。
 * 覆盖：百分之 / 小数点（数字点数字）/ 分之 / 含位值词的结构化整数。
 * 刻意不转「孤立单字数字」和「无位值词的逐位串」（如 一二三、三三零六），
 * 避免误伤成语、口语与号码。复杂/歧义场景建议开 AI 整理。
 */
export function convertChineseNumbers(text: string): string {
  if (!text) return text
  let result = text

  // 1. 百分之X → X%
  result = result.replace(
    new RegExp(`百分之([${NUM_CHARS}点]+)`, 'g'),
    (m: string, expr: string, offset: number, str: string) => {
      const val = parseNumberExpr(expr)
      return val === null ? m : maybeSpace(str, offset, `${val}%`)
    },
  )

  // 2. X分之Y → Y/X
  result = result.replace(
    new RegExp(`([${NUM_CHARS}]+)分之([${NUM_CHARS}]+)`, 'g'),
    (m: string, a: string, b: string) => {
      const av = parseChineseInteger(a)
      const bv = parseChineseInteger(b)
      return av === null || bv === null ? m : `${bv}/${av}`
    },
  )

  // 2.5 时间表达：必须在小数规则之前处理，避免「点」被当成小数点。
  //     时/分数字转阿拉伯，保留「点/分/半」标记（如 九点三十二分 → 9点32分）。
  const TIME_HOUR = '零一二两三四五六七八九十'
  const TIME_MIN = '零一二两三四五六七八九十'
  const DIGITS_NO_TEN = '零一二两三四五六七八九'

  // 合法小时：单字数字，或含「十」的复合（十 / 十二 / 二十三…），范围 0–24。
  // 借「含十」排除「一二三」这类逐位串（parseChineseInteger 对它们会给出误导性结果）。
  const validHour = (s: string): number | null => {
    if (s.length > 1 && !s.includes('十')) return null
    const h = parseChineseInteger(s)
    return h === null || h < 0 || h > 24 ? null : h
  }

  // 2.5a 时间：X点半 / X点Y分 / X点[含十的分钟]（九点半→9点半、九点三十二分→9点32分、九点三十→9点30）
  result = result.replace(
    new RegExp(
      `([${TIME_HOUR}]+)点(半|[${TIME_MIN}]+分(?!之)|[${DIGITS_NO_TEN}]*十[${DIGITS_NO_TEN}]*)`,
      'g',
    ),
    (m: string, hourStr: string, tail: string) => {
      const hour = validHour(hourStr)
      if (hour === null) return m
      if (tail === '半') return `${hour}点半`
      if (tail.endsWith('分')) {
        const min = parseChineseInteger(tail.slice(0, -1))
        return min === null || min < 0 || min > 59 ? m : `${hour}点${min}分`
      }
      // 含「十」的分钟、无「分」（九点三十 → 9点30）
      const min = parseChineseInteger(tail)
      return min === null || min < 0 || min > 59 ? m : `${hour}点${min}`
    },
  )

  // 2.5b 整点：X点（后面不跟数字/点）→ 9点（如 上午九点 → 上午9点）。
  //      「点」后跟数字的（三点一四）留给小数规则，避免误伤。
  //
  //      收紧：裸整点必须有明确「时间信号」才转，否则保留中文。目的是不把口语里
  //      表「a little bit」的「一点」误当成「1 点钟」——如「调宽一点」「有一点」
  //      「快一点」。时间信号（满足其一即转）：
  //        ① 前面紧邻时段词（上午 / 下午 / 晚上 …）；
  //        ② 后面紧跟「钟」（一点钟）；
  //        ③ 小时是含「十」的两位（十点 / 十二点，几乎只作时间用）。
  //      无信号时宁可不转，交给 AI 整理处理歧义场景（与整体「尽力而为」取向一致）。
  const TIME_OF_DAY_PREFIX = /(凌晨|清晨|早晨|早上|上午|中午|下午|傍晚|晚上|夜里|半夜|今早|今晚|明早|明晚|昨晚)$/
  result = result.replace(
    new RegExp(`([${TIME_HOUR}]+)点(?![${DIGIT_CHARS}点])`, 'g'),
    (m: string, hourStr: string, offset: number, str: string) => {
      const hour = validHour(hourStr)
      if (hour === null) return m
      const hasTenUnit = hourStr.includes('十')
      const hasClockSuffix = (str[offset + m.length] || '') === '钟'
      const hasTimePrefix = TIME_OF_DAY_PREFIX.test(str.slice(0, offset))
      if (!hasTenUnit && !hasClockSuffix && !hasTimePrefix) return m
      return maybeSpace(str, offset, `${hour}点`)
    },
  )

  // 3. 小数 / 多段点分数字：数字点数字(点数字)* → N.N 或 N.N.N
  //    单段是普通小数（三点一四 → 3.14）；多段是版本号等（零点一点零 → 0.1.0），每段逐位读。
  result = result.replace(
    new RegExp(`([${NUM_CHARS}]+)((?:点[${DIGIT_CHARS}]+)+)`, 'g'),
    (m: string, intp: string, rest: string, offset: number, str: string) => {
      // 护栏：小数尾数若紧跟位值词/时间词（说明被截断，或其实是时间/量词），不转，保留中文。
      // 例：九点三十（无「分」）→ 匹配到「九点三」，其后是「十」→ 跳过，宁留中文也不产出 9.3十。
      const after = str[offset + m.length] || ''
      if (after && '十百千万亿分时点刻秒'.includes(after)) return m
      const intVal = parseChineseInteger(intp)
      if (intVal === null) return m
      const groups = rest.split('点').filter((g: string) => g.length > 0)
      let out = String(intVal)
      for (const g of groups) {
        let dec = ''
        for (const ch of g) dec += String(CN_DIGITS[ch])
        out += `.${dec}`
      }
      return maybeSpace(str, offset, out)
    },
  )

  // 4. 结构化整数：长度≥2、含位值词、非黑名单
  result = result.replace(
    new RegExp(`[${NUM_CHARS}]+`, 'g'),
    (m: string, offset: number, str: string) => {
      if (m.length < 2 || !containsUnit(m) || INTEGER_BLACKLIST.has(m)) return m
      const val = parseChineseInteger(m)
      return val === null ? m : maybeSpace(str, offset, String(val))
    },
  )

  return result
}
// i18n-allow-end

/**
 * 还原被 ASR 拆开加空格的「无空格热词」（如豆包把热词 "SayIt" 识别成 "Say It"）。
 *
 * 仅处理不含空格、纯 ASCII 字母/数字的热词（中文不会被加空格）。对每个热词构造
 * 大小写敏感、字符间允许空白、两端带词边界的正则；命中后若与热词不同（即确实被
 * 拆开过）就合并还原成热词原形。
 *
 * 大小写敏感 + 词边界是为了避免误伤：普通小写短语（如句子里的 "say it"）不会被
 * 改成 "SayIt"，"Say Item" 也不会被错并成 "SayItem"。
 */
export function restoreHotwordSpacing(text: string, hotwords: string[]): string {
  if (!text || !hotwords || hotwords.length === 0) return text
  const targets = hotwords
    .map((w) => w.trim())
    .filter((w) => w.length >= 2 && /^[A-Za-z0-9]+$/.test(w) && /[A-Za-z]/.test(w))
  if (targets.length === 0) return text
  // 长词优先，避免短词先命中影响更长的热词
  targets.sort((a, b) => b.length - a.length)
  let result = text
  for (const hw of targets) {
    const inner = hw.split('').join('\\s*')
    // 大小写不敏感匹配（豆包可能把 "SayIt" 识别成 "Say it"，尾字母小写），
    // 但要求首字符大小写与热词一致，避免把普通小写短语（如 "say it"）误并成热词。
    const re = new RegExp(`\\b${inner}\\b`, 'gi')
    result = result.replace(re, (m) => {
      if (m === hw) return m
      // 连写命中（无内部空格，如 "typeless"/"sayit"）→ 直接还原成热词原样（含大小写）。
      // 带空格命中（如 "Say it"）→ 仅当首字母大小写一致才还原，避免把普通句子里的
      // 小写短语（如 "say it"）误并成热词。
      if (/\s/.test(m) && m[0] !== hw[0]) return m
      return hw
    })
  }
  return result
}

// ── 中文标点宽度规范化 ──

/**
 * 半角 → 全角的对应表。
 *
 * **刻意不含句号 `.`**：它的歧义代价太高。小数（3.14）、版本号（v1.2）、域名
 * （example.com）、文件名里全是半角句点，一旦误转就是破坏内容；而实测
 * whisper-large-v3-turbo 的中文句号本来就输出全角 `。`，不需要我们兜。
 * 宁可漏一个也不能错改一个。
 */
// i18n-allow-start: 标点符号本身就是这个函数处理的数据
const HALF_TO_FULL_WIDTH: Record<string, string> = {
  ',': '，',
  '?': '？',
  '!': '！',
  ';': '；',
  ':': '：',
}

/** 汉字区间。与 check-i18n.mjs 用的 CJK 判定保持一致。 */
const CJK_CHAR = /[\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff]/
// i18n-allow-end

/**
 * 把紧跟在汉字后面的半角标点转成全角。
 *
 * 为什么需要：Whisper（Groq 的 whisper-large-v3-turbo）转写中文时**混用两种宽度** ——
 * 句号给全角 `。`，逗号和问号给半角 `,` `?`。实测四种 prompt 加不给 prompt 输出
 * 一字不差，所以这是模型自身行为，靠请求参数改不动，只能在文本层归一。
 *
 * 判据只有一条：**前一个字符是汉字**。这一条把所有危险场景挡在外面 ——
 *   · `3,000` / `3.14`：前一个字符是数字，不动
 *   · `Hello, world`：前一个字符是字母，不动
 *   · `example.com` / `v1.2`：同上，且句号根本不在表里
 *   · `中文A,B`：逗号前是 `A`，不动
 * 而 `今天不错,我们走吧` 里两个逗号前面都是汉字，转成全角。
 *
 * 已知缺口（刻意保留）：标点夹在「数字/字母」和汉字之间时不转，例如
 * `一共100,很便宜` 里那个逗号会留半角。想覆盖它就必须放宽到「后一个字符是汉字」，
 * 而那会连 `比例3:2中文`、`3,000元` 一起改掉。少一个全角逗号只是观感问题，
 * 改坏数字是内容损坏 —— 宁可漏改。
 *
 * 中文供应商（豆包 / 千问 / MiMo）本来就输出全角，纯英文文本里没有汉字，
 * 所以对它们这个函数是彻底的空操作 —— 这也是它不需要做成开关的原因：
 * 它不是排版偏好，是把一种明确的错误宽度纠正过来。
 */
export function normalizeChinesePunctuation(text: string): string {
  if (!text) return text
  let result = ''
  for (let i = 0; i < text.length; i++) {
    const ch = text[i]
    const full = HALF_TO_FULL_WIDTH[ch]
    // 回看用的是原文的前一个字符：转换不会改变「是不是汉字」，所以不必看已产出的结果
    result += full && CJK_CHAR.test(text[i - 1] ?? '') ? full : ch
  }
  return result
}

/** 去除每行末尾的标点符号（及尾随空白）。 */
export function stripTrailingPunctuation(text: string): string {
  if (!text) return text
  return text
    .split('\n')
    .map((line) => line.replace(/[。．.，,、；;：:！!？?…⋯～~—－\s]+$/u, '')) // i18n-allow: 中文标点数据
    .join('\n')
}

/**
 * 将所有标点符号替换为空格，并折叠多余空格（保留换行）。
 * 保留数字间的小数点（3.14）、百分号（15%）和斜杠（2/5），避免破坏数字规范化的产物。
 */
export function replacePunctuationWithSpace(text: string): string {
  if (!text) return text
  let result = text.replace(
    /(?<!\d)\.(?!\d)|[，。、；：！？…⋯""''‘’“”（）《》〈〉「」『』【】—－~～·!?,;:"'`()\[\]{}<>_=+|\\@#^&*-]/gu, // i18n-allow: 中文标点数据
    ' ',
  )
  result = result.replace(/[^\S\n]+/g, ' ').replace(/ *\n */g, '\n').trim()
  return result
}

// ── 设置持久化 ──

export interface TextPostProcessOptions {
  /** 智能分段（仅在关闭 AI 整理的极速模式下生效），默认开启 */
  autoSegment: boolean
  /** 数字规范化 */
  normalizeNumbers: boolean
  /** 去除句末标点 */
  stripTrailingPunctuation: boolean
  /** 标点符号替换为空格 */
  punctuationToSpace: boolean
}

const STORAGE_KEY = 'textPostProcess'

export const DEFAULT_POST_PROCESS: TextPostProcessOptions = {
  autoSegment: true,
  normalizeNumbers: true,
  stripTrailingPunctuation: false,
  punctuationToSpace: false,
}

export async function getTextPostProcessOptions(): Promise<TextPostProcessOptions> {
  const saved = await getSetting<Partial<TextPostProcessOptions>>(STORAGE_KEY, {})
  return { ...DEFAULT_POST_PROCESS, ...(saved || {}) }
}

export async function saveTextPostProcessOptions(opts: TextPostProcessOptions): Promise<void> {
  await setSetting(STORAGE_KEY, opts)
}

export interface ApplyTransformsOptions {
  /**
   * 这段文本是不是**未经 AI 整理的纯 ASR 结果**。
   *
   * 「格式规范」（智能分段 / 数字规范化 / 去句末标点 / 标点转空格）是我们在没有 AI 时
   * 兜底做的排版。一旦 AI 整理介入，格式就该由 AI 的规则统一负责——两边都做会打架
   * （比如 AI 已经排好版，我们又去掉了它的句末句号）。所以这四项只在 rawAsr 时生效。
   *
   * 文本替换不受此开关影响，任何时候都执行：它是用户点名要替换的固定词
   * （改公司名、术语纠正等），和"谁来排版"无关，AI 也未必会照做。
   *
   * 默认 true：调用方没说明时按纯 ASR 处理（更安全，不会漏掉该做的规范化）。
   */
  rawAsr?: boolean
}

/**
 * 统一入口。顺序：中文标点宽度归一 → 智能分段 → 数字规范化 → 用户替换规则
 * → 去句末标点 → 标点转空格。
 * 其中除「用户替换规则」外都属于「格式规范」，仅在 rawAsr（无 AI 整理）时执行。
 */
export async function applyTextTransforms(
  text: string,
  options: ApplyTransformsOptions = {},
): Promise<string> {
  if (!text) return text
  const opts = await getTextPostProcessOptions()
  // AI 整理过的文本，格式交给 AI；我们只保留文本替换。
  const ownFormat = options.rawAsr ?? true
  let result = text
  // 排在最前面：后面几步都在读标点做判断（分段找句末、去句末标点），
  // 让它们看到统一宽度的输入，比让每一步各自兼容两种宽度可靠。
  // 没有对应开关：它纠正的是一种明确的错误宽度，不是可选的排版偏好。
  if (ownFormat) result = normalizeChinesePunctuation(result)
  if (ownFormat && opts.autoSegment) result = segmentAsrText(result)
  if (ownFormat && opts.normalizeNumbers) result = convertChineseNumbers(result)
  result = await applyTextReplacements(result)
  if (ownFormat && opts.stripTrailingPunctuation) result = stripTrailingPunctuation(result)
  if (ownFormat && opts.punctuationToSpace) result = replacePunctuationWithSpace(result)
  return result
}
