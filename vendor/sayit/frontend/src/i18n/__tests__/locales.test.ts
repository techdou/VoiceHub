import { describe, expect, it, afterEach } from 'vitest'
import en from '../locales/en.json'
import zhCN from '../locales/zh-CN.json'
import {
  getLocale,
  isLocale,
  normalizePreference,
  resolveLocale,
  setLocale,
  t,
  LOCALES,
} from '..'
import {
  appPromptRuleDisplayName,
  historyFailureReasonDisplay,
  localModelDisplayDescription,
  localModelDisplayLanguages,
  localModelDisplayName,
  promptPresetDisplayName,
  recordedAppDisplayName,
  recordedPromptPresetDisplayName,
} from '../displayNames'

/**
 * 这些测试锁的是 i18n 的**不变量**，不是具体文案。
 * 断言里出现任何一句人类语言时都要先问：这条会不会在文案改一个字之后就挂？
 */
describe('locale 表', () => {
  it('en 与 zh-CN 的 key 集合完全一致', () => {
    // 类型层面已经保证 en 不缺 key（i18n/index.ts 的 Record<TranslationKey, string>），
    // 这里补的是另一半：en 里**多出来**的 key 类型查不出来，只能靠测试。
    const zhKeys = Object.keys(zhCN).sort()
    const enKeys = Object.keys(en).sort()
    expect(enKeys).toEqual(zhKeys)
  })

  it('没有空文案', () => {
    for (const [locale, table] of [['zh-CN', zhCN], ['en', en]] as const) {
      for (const [key, value] of Object.entries(table)) {
        expect(value.trim(), `${locale} 的 ${key} 是空的`).not.toBe('')
      }
    }
  })

  it('两边的插值占位符一一对应', () => {
    // 漏掉一个 {name} 不会报错，只会在界面上少显示一个东西 —— 典型的静默失败。
    const placeholders = (text: string) => (text.match(/\{(\w+)\}/g) ?? []).sort()
    for (const key of Object.keys(zhCN) as (keyof typeof zhCN)[]) {
      expect(placeholders(en[key]), `key=${key}`).toEqual(placeholders(zhCN[key]))
    }
  })
})

describe('resolveLocale', () => {
  it('所有中文变体都落到简体（不把繁体用户推去英文）', () => {
    for (const tag of ['zh', 'zh-CN', 'zh-TW', 'zh-Hans', 'ZH-HK']) {
      expect(resolveLocale(tag), tag).toBe('zh-CN')
    }
  })

  it('其余语言与空值一律落到英文', () => {
    for (const tag of ['en', 'en-US', 'ja-JP', 'ko-KR', 'de', '', null, undefined]) {
      expect(resolveLocale(tag), String(tag)).toBe('en')
    }
  })
})

describe('normalizePreference', () => {
  it('保留合法值', () => {
    expect(normalizePreference('auto')).toBe('auto')
    expect(normalizePreference('en')).toBe('en')
    expect(normalizePreference('zh-CN')).toBe('zh-CN')
  })

  it('脏数据回落 auto', () => {
    for (const value of ['zh', 'EN', '', null, undefined, 42, {}]) {
      expect(normalizePreference(value), String(value)).toBe('auto')
    }
  })
})

describe('t', () => {
  afterEach(() => setLocale('zh-CN'))

  // 断言对照 JSON 表本身，不写死具体文案：改一个字就挂的测试没人愿意维护，
  // 而这里要锁的是「t 有没有按当前语言查表」这个机制。
  it('按当前语言取文案', () => {
    setLocale('zh-CN')
    expect(t('nav.home')).toBe(zhCN['nav.home'])
    setLocale('en')
    expect(t('nav.home')).toBe(en['nav.home'])
    expect(en['nav.home']).not.toBe(zhCN['nav.home'])
  })

  it('替换插值占位符', () => {
    setLocale('en')
    const rendered = t('titleBar.presetTooltip', { name: 'Faithful' })
    expect(rendered).toBe(en['titleBar.presetTooltip'].replace('{name}', 'Faithful'))
    expect(rendered).not.toContain('{name}')
  })

  it('未提供的占位符原样保留，不吐 undefined', () => {
    setLocale('en')
    expect(t('titleBar.presetTooltip')).toContain('{name}')
    expect(t('titleBar.presetTooltip')).not.toContain('undefined')
  })
})

describe('内置名称显示', () => {
  afterEach(() => setLocale('zh-CN'))

  it('内置 preset 与应用规则按稳定 id 切换语言', () => {
    const preset = { id: 'intent', name: 'legacy name', builtin: true }
    const rule = { id: 'notepad', appId: 'notepad', name: 'legacy name', builtin: true }

    setLocale('zh-CN')
    expect(promptPresetDisplayName(preset)).toBe(zhCN['builtinPreset.intent'])
    expect(appPromptRuleDisplayName(rule)).toBe(zhCN['builtinApp.notepad'])

    setLocale('en')
    expect(promptPresetDisplayName(preset)).toBe(en['builtinPreset.intent'])
    expect(appPromptRuleDisplayName(rule)).toBe(en['builtinApp.notepad'])
    expect(recordedPromptPresetDisplayName('intent', 'legacy name')).toBe(en['builtinPreset.intent'])
    expect(recordedAppDisplayName('notepad', 'legacy name')).toBe(en['builtinApp.notepad'])
  })

  it('用户自建名称与未知历史名称保持原样', () => {
    setLocale('en')
    expect(promptPresetDisplayName({ id: 'custom', name: 'My preset', builtin: false })).toBe('My preset')
    expect(appPromptRuleDisplayName({ id: 'slack', appId: 'slack', name: 'My Slack', builtin: false })).toBe('My Slack')
    expect(recordedPromptPresetDisplayName('custom', 'My preset')).toBe('My preset')
    expect(recordedAppDisplayName('slack', 'My Slack')).toBe('My Slack')
  })
})

describe('本地模型目录显示', () => {
  afterEach(() => setLocale('zh-CN'))

  it('已知模型按稳定 id 切换名称、简介和语种', () => {
    const model = {
      id: 'qwen3-asr-1.7b-gguf',
      name: 'raw name',
      description: 'raw description',
      languages_label: 'raw languages',
    }

    setLocale('zh-CN')
    expect(localModelDisplayName(model)).toBe(zhCN['localModel.qwen17Accurate.name'])
    expect(localModelDisplayDescription(model)).toBe(zhCN['localModel.qwen17Accurate.description'])
    expect(localModelDisplayLanguages(model)).toBe(zhCN['localModel.qwen17Accurate.languages'])

    setLocale('en')
    expect(localModelDisplayName(model)).toBe(en['localModel.qwen17Accurate.name'])
    expect(localModelDisplayDescription(model)).toBe(en['localModel.qwen17Accurate.description'])
    expect(localModelDisplayLanguages(model)).toBe(en['localModel.qwen17Accurate.languages'])
  })

  it('未知或用户侧载模型保留目录原文', () => {
    setLocale('en')
    const model = {
      id: 'custom-local-model',
      name: 'Custom model',
      description: 'Custom description',
      languages_label: 'Custom languages',
    }
    expect(localModelDisplayName(model)).toBe(model.name)
    expect(localModelDisplayDescription(model)).toBe(model.description)
    expect(localModelDisplayLanguages(model)).toBe(model.languages_label)
  })
})

describe('历史失败原因显示', () => {
  afterEach(() => setLocale('zh-CN'))

  it('新记录按稳定 code 跟随当前界面语言', () => {
    const record = { failReasonCode: 'provider_bad_key' as const, failReason: '底层原文' }

    setLocale('zh-CN')
    expect(historyFailureReasonDisplay(record)).toBe(zhCN['err.provider.badKey'])
    setLocale('en')
    expect(historyFailureReasonDisplay(record)).toBe(en['err.provider.badKey'])
  })

  it('老记录没有 code 时原样显示', () => {
    setLocale('en')
    expect(historyFailureReasonDisplay({ failReason: '旧版失败原因' })).toBe('旧版失败原因')
  })
})

describe('locale 状态', () => {
  afterEach(() => setLocale('zh-CN'))

  it('isLocale 只认受支持的值', () => {
    expect(LOCALES.every(isLocale)).toBe(true)
    for (const value of ['zh', 'en-US', '', null, 1]) {
      expect(isLocale(value), String(value)).toBe(false)
    }
  })

  it('setLocale 忽略非法值', () => {
    setLocale('en')
    // @ts-expect-error 故意传非法值：运行时来源（overlay payload）不受类型保护
    setLocale('fr')
    expect(getLocale()).toBe('en')
  })
})
