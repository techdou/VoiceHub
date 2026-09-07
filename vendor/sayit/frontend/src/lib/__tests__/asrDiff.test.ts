import { describe, expect, it } from 'vitest'
import { computeAsrDiff, countAsrDiffChanges, hasAsrDiffChange, normalizeForDiff } from '../asrDiff'

/** 把 segments 还原成两侧文本，用来证明没丢字。 */
function rebuild(segments: ReturnType<typeof computeAsrDiff>) {
  const left = segments.filter((s) => s.kind !== 'insert').map((s) => s.text).join('')
  const right = segments.filter((s) => s.kind !== 'delete').map((s) => s.text).join('')
  return { left, right }
}

describe('computeAsrDiff', () => {
  it('相同文本没有改动', () => {
    const segments = computeAsrDiff('今天天气很好', '今天天气很好')
    expect(hasAsrDiffChange(segments)).toBe(false)
    expect(countAsrDiffChanges(segments)).toBe(0)
  })

  it('只有行尾/首尾空白不同也算没有改动', () => {
    expect(hasAsrDiffChange(computeAsrDiff('第一行\n第二行', '第一行\r\n第二行'))).toBe(false)
    expect(hasAsrDiffChange(computeAsrDiff('你好', '  你好  '))).toBe(false)
  })

  it('中文改一个字只标出那个字', () => {
    const segments = computeAsrDiff('明天会议改到周二', '明天会议改到周三')
    expect(hasAsrDiffChange(segments)).toBe(true)
    expect(segments.filter((s) => s.kind === 'delete').map((s) => s.text)).toEqual(['二'])
    expect(segments.filter((s) => s.kind === 'insert').map((s) => s.text)).toEqual(['三'])
  })

  it('两侧文本可以从 segments 无损还原', () => {
    const original = '我们用的是 sayit 这个工具，识别率还不错'
    const corrected = '我们用的是 SayIt 这个工具，识别率还不错。'
    const segments = computeAsrDiff(original, corrected)
    const { left, right } = rebuild(segments)
    expect(left).toBe(normalizeForDiff(original))
    expect(right).toBe(normalizeForDiff(corrected))
  })

  it('英文改动不会摊成一大片碎片', () => {
    // 语义整理（fast-diff 的第 4 个参数）负责把改动收拢：
    // 单词内部改一个字母时它不会扩展成整词（recogni|z→s|e 是预期的、也读得懂），
    // 但**不能**出现"每隔一两个字符就交替一次"的碎片流 —— 那种才没法看。
    const segments = computeAsrDiff('please recognize this word', 'please recognise that word')
    expect(segments.length).toBeLessThanOrEqual(8)
    // 未改动的部分必须整段保留，而不是被切碎
    expect(segments.some((s) => s.kind === 'equal' && s.text.includes('please recogni'))).toBe(true)
    expect(segments.some((s) => s.kind === 'equal' && s.text.includes('word'))).toBe(true)
  })

  it('多处改动分别计数', () => {
    const segments = computeAsrDiff('张三和李四明天开会', '张五和李六明天开会')
    expect(countAsrDiffChanges(segments)).toBe(2)
  })

  it('纯新增与纯删除', () => {
    expect(computeAsrDiff('好', '好的').filter((s) => s.kind === 'insert')).toHaveLength(1)
    expect(computeAsrDiff('好的', '好').filter((s) => s.kind === 'delete')).toHaveLength(1)
  })

  it('空文本不产生段', () => {
    expect(computeAsrDiff('', '')).toEqual([])
  })

  it('不产生空文本段', () => {
    const segments = computeAsrDiff('abc def', 'abc xyz def')
    expect(segments.every((s) => s.text.length > 0)).toBe(true)
  })
})
