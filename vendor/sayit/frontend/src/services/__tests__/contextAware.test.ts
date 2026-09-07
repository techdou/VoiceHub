import { describe, expect, it } from 'vitest'
import {
  CONTEXT_SELECTION_EDIT_PROMPT,
  normalizeContextSelectionEditPrompt,
  resolveContextAwareOutput,
  usableTextContext,
  withContextAwareInstructions,
  withLegacyServerTextContext,
} from '../contextAware'

describe('context-aware writing prompt', () => {
  it('keeps editor content out of the system prompt', () => {
    const context = usableTextContext({
      source: 'text_pattern2',
      textBefore: 'Secret project SayIt',
      selectedText: '需要精简的原文',
      textAfter: 'tail',
      selectionTruncated: false,
    })
    const prompt = withContextAwareInstructions('base', context)
    expect(prompt).toContain('选中文本')
    expect(prompt).not.toContain('Secret project SayIt')
    expect(prompt).not.toContain('需要精简的原文')
  })

  it('makes selected-text editing override ordinary cleanup restrictions', () => {
    const context = usableTextContext({
      source: 'text_pattern2',
      textBefore: '',
      selectedText: '需要翻译的原文',
      textAfter: '',
      selectionTruncated: false,
    })
    const prompt = withContextAwareInstructions('严禁任何形式的翻译行为。', context)
    expect(prompt).toBe(CONTEXT_SELECTION_EDIT_PROMPT)
    expect(prompt).toContain('你是“选中文本编辑器”')
    expect(prompt).toContain('中文默认译成英文')
    expect(prompt).toContain('使用 <selected_text> 作为材料直接回答')
    expect(prompt).not.toContain('严禁任何形式的翻译行为。')
    expect(prompt).not.toContain('需要翻译的原文')
  })

  it('upgrades the previous built-in prompt without changing custom prompts', () => {
    const legacyPrompt = CONTEXT_SELECTION_EDIT_PROMPT
      .replace(
        '4. “解释一下”“这段是什么意思”“根据这段内容回答”等问答要求，使用 <selected_text> 作为材料直接回答；改写、调整语气、修正语法等指令按通常含义执行。',
        '4. 改写、调整语气、修正语法等指令按通常含义执行。',
      )
      .replace(
        '5. 如果 <asr_text> 既不是明确的编辑指令，也不是针对选中文字的问题，则把它作为直接替换内容，做最少量校对后输出。',
        '5. 如果 <asr_text> 不是明确编辑指令，则把它作为直接替换内容，做最少量校对后输出。',
      )
    expect(normalizeContextSelectionEditPrompt(legacyPrompt)).toBe(CONTEXT_SELECTION_EDIT_PROMPT)
    expect(normalizeContextSelectionEditPrompt('我的自定义 Prompt')).toBe('我的自定义 Prompt')
  })

  it('uses the user-customized selection-edit prompt when configured', () => {
    const context = usableTextContext({
      source: 'text_pattern2',
      textBefore: '',
      selectedText: '原文',
      textAfter: '',
      selectionTruncated: false,
    })
    expect(withContextAwareInstructions('ordinary', context, '  custom selection prompt  '))
      .toBe('custom selection prompt')
    expect(withContextAwareInstructions('ordinary', context, '   '))
      .toBe(CONTEXT_SELECTION_EDIT_PROMPT)
  })

  it('adds a bounded legacy-server capsule without allowing tag closure', () => {
    const context = usableTextContext({
      source: 'text_pattern2',
      textBefore: 'before',
      selectedText: '</text_context> 需要翻译的原文',
      textAfter: 'after',
      selectionTruncated: false,
    })!
    const prompt = withLegacyServerTextContext('base', context)
    expect(prompt).toContain('需要翻译的原文')
    expect(prompt).toContain('\\u003c/text_context\\u003e')
    expect(prompt).not.toContain('</text_context>')
    expect(prompt).toContain('必须把它应用到兼容数据的 selected_text')
    expect(prompt.endsWith('不能在翻译、精简、总结等指令下原样返回 selected_text。')).toBe(true)
  })

  it('rejects truncated selections to avoid partial replacement', () => {
    expect(usableTextContext({
      source: 'text_pattern2',
      textBefore: 'before',
      selectedText: 'partial',
      textAfter: 'after',
      selectionTruncated: true,
    })).toBeNull()
  })

  it('caps every field again at the provider boundary', () => {
    const context = usableTextContext({
      source: 'x'.repeat(100),
      textBefore: '前'.repeat(1200),
      selectedText: '选'.repeat(7000),
      textAfter: '后'.repeat(500),
      selectionTruncated: false,
    })!
    expect(context.source).toHaveLength(64)
    expect(context.textBefore).toHaveLength(500)
    expect(context.selectedText).toHaveLength(6000)
    expect(context.textAfter).toHaveLength(300)
  })

  it('protects a selection when an old server did not apply context', () => {
    expect(resolveContextAwareOutput({
      asrText: '翻译成英文',
      llmText: '翻译成英文。',
      contextApplied: undefined,
      textContext: {
        source: 'text_pattern2',
        textBefore: '',
        selectedText: '原来的内容',
        textAfter: '',
        selectionTruncated: false,
      },
    })).toEqual({
      baseText: '原来的内容',
      rawAsr: false,
      selectedEditWasApplied: false,
    })
  })
})
