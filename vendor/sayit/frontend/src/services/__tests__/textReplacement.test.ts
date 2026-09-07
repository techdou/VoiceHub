import { describe, it, expect } from 'vitest'
import { applyReplacements, parseBatchReplacements, type TextReplacementRule } from '../textReplacement'

function rule(from: string, to: string, enabled = true): TextReplacementRule {
  return { id: '1', from, to, enabled }
}

describe('applyReplacements', () => {
  it('替换匹配的文本', () => {
    const rules = [rule('你好', 'Hello')]
    expect(applyReplacements('你好世界', rules)).toBe('Hello世界')
  })

  it('替换多次出现', () => {
    const rules = [rule('啊', '')]
    expect(applyReplacements('啊这个啊那个啊', rules)).toBe('这个那个')
  })

  it('禁用的规则不生效', () => {
    const rules = [rule('你好', 'Hello', false)]
    expect(applyReplacements('你好世界', rules)).toBe('你好世界')
  })

  it('空 from 不替换', () => {
    const rules = [rule('', 'Hello')]
    expect(applyReplacements('你好世界', rules)).toBe('你好世界')
  })

  it('多条规则按顺序执行', () => {
    const rules = [
      rule('A', 'B'),
      rule('B', 'C'),
    ]
    // A → B → C（链式替换）
    expect(applyReplacements('A', rules)).toBe('C')
  })

  it('空规则列表返回原文', () => {
    expect(applyReplacements('你好', [])).toBe('你好')
  })

  it('空文本返回空', () => {
    expect(applyReplacements('', [rule('a', 'b')])).toBe('')
  })
})

describe('parseBatchReplacements', () => {
  it('英文逗号分隔', () => {
    expect(parseBatchReplacements('安卓说话,按住说话')).toEqual([
      { from: '安卓说话', to: '按住说话' },
    ])
  })

  it('中文逗号分隔', () => {
    expect(parseBatchReplacements('安卓说话，按住说话')).toEqual([
      { from: '安卓说话', to: '按住说话' },
    ])
  })

  it('制表符分隔（从表格粘贴）', () => {
    expect(parseBatchReplacements('Cloud Code\tClaude Code')).toEqual([
      { from: 'Cloud Code', to: 'Claude Code' },
    ])
  })

  it('=> 与 -> 分隔', () => {
    expect(parseBatchReplacements('a => b\nc -> d')).toEqual([
      { from: 'a', to: 'b' },
      { from: 'c', to: 'd' },
    ])
  })

  it('多行、忽略空行、去除首尾空白', () => {
    const input = ' 原文1, 替换1 \n\n原文2,替换2\n'
    expect(parseBatchReplacements(input)).toEqual([
      { from: '原文1', to: '替换1' },
      { from: '原文2', to: '替换2' },
    ])
  })

  it('无分隔符的行：替换为留空（删除）', () => {
    expect(parseBatchReplacements('嗯')).toEqual([{ from: '嗯', to: '' }])
  })

  it('取最靠前的分隔符，替换内容可含其它逗号', () => {
    expect(parseBatchReplacements('abc,a, b')).toEqual([{ from: 'abc', to: 'a, b' }])
  })

  it('原文为空的行忽略', () => {
    expect(parseBatchReplacements(',只有替换')).toEqual([])
  })
})

describe('applyReplacements 的顺序语义', () => {
  const rule = (id: string, from: string, to: string) => ({ id, from, to, enabled: true })

  it('顺序即执行顺序：前一条的结果会参与后一条的匹配（级联）', () => {
    // A: 甲 → 乙，B: 乙 → 丙。A 在前时会一路级联成"丙"
    const cascade = applyReplacements('甲', [rule('a', '甲', '乙'), rule('b', '乙', '丙')])
    expect(cascade).toBe('丙')

    // 换成 B 在前，"乙 → 丙"先跑（此时还没有"乙"），结果停在"乙"
    const noCascade = applyReplacements('甲', [rule('b', '乙', '丙'), rule('a', '甲', '乙')])
    expect(noCascade).toBe('乙')
  })

  it('两条规则匹配同一段文本时，靠前的先生效', () => {
    const first = applyReplacements('安卓说话', [
      rule('a', '安卓说话', '按住说话'),
      rule('b', '安卓', 'Android'),
    ])
    expect(first).toBe('按住说话')

    const second = applyReplacements('安卓说话', [
      rule('b', '安卓', 'Android'),
      rule('a', '安卓说话', '按住说话'),
    ])
    expect(second).toBe('Android说话')
  })

  it('禁用的规则不参与，也不影响其余规则的顺序', () => {
    const result = applyReplacements('甲', [
      { ...rule('a', '甲', '乙'), enabled: false },
      rule('b', '甲', '丙'),
    ])
    expect(result).toBe('丙')
  })
})
