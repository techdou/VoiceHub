import { beforeEach, describe, expect, it, vi } from 'vitest'

const bridgeState = vi.hoisted(() => ({
  values: new Map<string, unknown>(),
}))

vi.mock('@/services/bridge', () => ({
  storeGet: vi.fn(async (key: string) => bridgeState.values.get(key) ?? null),
  storeSet: vi.fn(async (key: string, value: unknown) => {
    bridgeState.values.set(key, value)
  }),
}))

import { getDefault } from '../defaults'
import {
  BUILTIN_PRESETS,
  builtinPromptContentHash,
  getBuiltinPromptPresets,
  getPromptPresets,
  normalizeBuiltinPromptLanguage,
  savePromptPreset,
  setBuiltinPromptLanguage,
  type PromptPreset,
} from '../store'

describe('内置 Prompt 语言', () => {
  beforeEach(() => bridgeState.values.clear())

  it('默认使用中文，非法持久化值也安全回落到中文', () => {
    expect(getDefault('ai.builtinPromptLanguage')).toBe('zh-CN')
    expect(normalizeBuiltinPromptLanguage('zh-CN')).toBe('zh-CN')
    expect(normalizeBuiltinPromptLanguage('en')).toBe('en')
    for (const value of ['', 'zh', 'en-US', null, undefined, 1]) {
      expect(normalizeBuiltinPromptLanguage(value), String(value)).toBe('zh-CN')
    }
  })

  it('中英文定义的 id 和顺序一致，且四份英文 Prompt 均已提供', () => {
    const zh = getBuiltinPromptPresets('zh-CN')
    const en = getBuiltinPromptPresets('en')

    expect(zh.map((preset) => preset.id)).toEqual(BUILTIN_PRESETS.map((preset) => preset.id))
    expect(en.map((preset) => preset.id)).toEqual(zh.map((preset) => preset.id))
    expect(en).toHaveLength(4)
    for (let index = 0; index < en.length; index += 1) {
      expect(en[index].systemPrompt.trim()).not.toBe('')
      expect(en[index].systemPrompt).not.toBe(zh[index].systemPrompt)
      expect(en[index].builtinPromptLanguage).toBe('en')
      expect(zh[index].builtinPromptLanguage).toBe('zh-CN')
    }
  })

  it('持久化选择后按对应语言加载', async () => {
    await setBuiltinPromptLanguage('en')
    const presets = await getPromptPresets()

    expect(bridgeState.values.get('ai.builtinPromptLanguage')).toBe('en')
    expect(presets.every((preset) => !preset.builtin || preset.builtinPromptLanguage === 'en')).toBe(true)
  })

  it('中英文内置修改分开保存，恢复默认只清理当前语言', async () => {
    const zhIntent = getBuiltinPromptPresets('zh-CN')[0]
    const enIntent = getBuiltinPromptPresets('en')[0]

    await savePromptPreset({ ...zhIntent, systemPrompt: '中文自定义 Prompt' })
    await savePromptPreset({ ...enIntent, systemPrompt: 'Custom English prompt' })

    const stored = bridgeState.values.get('promptPresets') as PromptPreset[]
    expect(stored.find((preset) => preset.builtinPromptLanguage === 'zh-CN')?.builtinPromptBaseHash)
      .toBe(builtinPromptContentHash(zhIntent.systemPrompt))
    expect(stored.find((preset) => preset.builtinPromptLanguage === 'en')?.builtinPromptBaseHash)
      .toBe(builtinPromptContentHash(enIntent.systemPrompt))

    expect((await getPromptPresets('zh-CN'))[0].systemPrompt).toBe('中文自定义 Prompt')
    expect((await getPromptPresets('en'))[0].systemPrompt).toBe('Custom English prompt')

    await savePromptPreset(zhIntent)
    expect((await getPromptPresets('zh-CN'))[0].systemPrompt).toBe(zhIntent.systemPrompt)
    expect((await getPromptPresets('en'))[0].systemPrompt).toBe('Custom English prompt')
  })

  it('旧版无语言字段的 override 只归入中文', async () => {
    const legacyOverride: PromptPreset = {
      id: 'intent',
      name: '旧名称',
      systemPrompt: '旧中文 Prompt',
    }
    bridgeState.values.set('promptPresets', [legacyOverride])

    expect((await getPromptPresets('zh-CN'))[0].systemPrompt).toBe(legacyOverride.systemPrompt)
    expect((await getPromptPresets('zh-CN'))[0].builtinPromptModified).toBe(true)
    expect((await getPromptPresets('zh-CN'))[0].builtinPromptUpdateAvailable).toBe(false)
    expect((await getPromptPresets('en'))[0].systemPrompt)
      .toBe(getBuiltinPromptPresets('en')[0].systemPrompt)
  })

  it('保存基线与当前内置定义不同时提示有更新', async () => {
    bridgeState.values.set('promptPresets', [{
      id: 'intent',
      name: 'Intent cleanup',
      systemPrompt: 'My customized prompt',
      builtinPromptLanguage: 'en',
      builtinPromptBaseHash: 'older-version',
    } satisfies PromptPreset])

    const preset = (await getPromptPresets('en'))[0]
    expect(preset.builtinPromptModified).toBe(true)
    expect(preset.builtinPromptUpdateAvailable).toBe(true)
  })

  it('读取时清理与当前官方内容完全相同的旧快照', async () => {
    const current = getBuiltinPromptPresets('zh-CN')[0]
    bridgeState.values.set('promptPresets', [{
      id: current.id,
      name: '旧名称',
      systemPrompt: current.systemPrompt,
    } satisfies PromptPreset])

    const presets = await getPromptPresets('zh-CN')
    expect(presets[0].builtinPromptModified).toBeUndefined()
    expect(bridgeState.values.get('promptPresets')).toEqual([])
  })
})
