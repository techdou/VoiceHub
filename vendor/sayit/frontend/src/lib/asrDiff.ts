import diff from 'fast-diff'

/**
 * ASR 纠错面板用的 diff：原始识别文本 → 用户修正后的文本。
 *
 * 渲染层不参与计算，是为了能在没有 jsdom 的环境里单测（本项目 vitest 没装 jsdom）。
 *
 * 用 fast-diff 而不是自己写：它就是 diff-match-patch 的算法，自带
 * `cleanupSemantic` / `cleanupSemanticLossless`，会把改动边界往空白和标点上挪。
 * 这对英文很关键 —— 纯字符级 diff 会把 "recognize"→"recognise" 切成
 * 「recogni + s/z + e」这种碎片，读起来像乱码。中文没有词边界，天然按字，正好。
 */
export type AsrDiffKind = 'equal' | 'delete' | 'insert'

export interface AsrDiffSegment {
  kind: AsrDiffKind
  text: string
}

/** 统一行尾并去掉首尾空白：只有行尾不同不该显示成"有改动"。 */
export function normalizeForDiff(text: string): string {
  return text.replace(/\r\n/g, '\n').replace(/\r/g, '\n').trim()
}

export function computeAsrDiff(original: string, corrected: string): AsrDiffSegment[] {
  const a = normalizeForDiff(original)
  const b = normalizeForDiff(corrected)
  if (a === b) return a ? [{ kind: 'equal', text: a }] : []

  // 第 4 个参数 = 语义整理，别省
  return diff(a, b, undefined, true)
    .filter(([, text]) => text.length > 0)
    .map(([op, text]): AsrDiffSegment => ({
      kind: op === diff.INSERT ? 'insert' : op === diff.DELETE ? 'delete' : 'equal',
      text,
    }))
}

/** 改动处数（连续的一删一插算一处），用于给用户一个"改了几处"的量感。 */
export function countAsrDiffChanges(segments: AsrDiffSegment[]): number {
  let count = 0
  let inChange = false
  for (const segment of segments) {
    if (segment.kind === 'equal') {
      inChange = false
      continue
    }
    if (!inChange) {
      count += 1
      inChange = true
    }
  }
  return count
}

export function hasAsrDiffChange(segments: AsrDiffSegment[]): boolean {
  return segments.some((segment) => segment.kind !== 'equal')
}
