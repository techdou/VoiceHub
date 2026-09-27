import { describe, expect, it } from 'vitest'

// 源码当字符串读进来（Vite 的 `?raw`）。用它而不是 node:fs 是因为本项目没装
// @types/node，而 `vite/client` 已经声明了 `*?raw` —— 与 hotwordCapability.test.ts 同款。
import SOURCE from '../RecorderOrchestrator.ts?raw'

/**
 * 「识别没出文字」不许弹卡片 —— 用读源码的方式钉住。
 *
 * 为什么不用行为测试：这个判据在 RecorderOrchestrator 里**有三条独立路径**各走一遍
 * （onAsr 的空结果、onFinal 的空结果、onDone 的协议异常），而它们各自要一整套
 * provider / WebSocket / 历史存档的桩才能跑起来。2026-09-23 的实际代价是：只改了
 * 其中一条，另一条照旧弹卡片，用户原样复现 —— 而当时改动配的单测（OverlayService
 * 层面的 showNoSpeech 只发 toast）是全绿的，因为它压根没覆盖到漏掉的那条路径。
 *
 * 所以这里直接断言**源码里不存在那种写法**：任何新增的第四条路径也会被它抓住。
 */
/** 弹卡片的两个入口。`failRunWithCard` 内部就是 showFailure + 存档。 */
const CARD_ENTRY_POINTS = ['showFailure(', 'failRunWithCard(']

/**
 * 允许弹卡片的标题 key。**加新 key 进来之前先问一句：这件事需要用户去做点什么吗？**
 * 不需要的就该走 showNoSpeech。
 *
 * - emptyAfterProcessing：识别出字了、被用户自己的替换规则清空 → 要他去改规则
 * - protocolIncomplete：服务端结束会话却没回结果 → 确实出错了
 * - connectionLost / processingTimeout / recognitionFailed：同上
 */
const CARD_TITLE_KEYS = [
  'recorder.emptyAfterProcessingTitle',
  'recorder.protocolIncompleteTitle',
  'recorder.connectionLostTitle',
  'recorder.processingTimeoutTitle',
  'recorder.recognitionFailedTitle',
]

describe('识别没出文字不弹卡片', () => {
  it('源码读到了，不是空字符串（否则下面几条会假绿）', () => {
    expect(SOURCE.length).toBeGreaterThan(1000)
    expect(SOURCE).toContain('showNoSpeech')
  })

  it('卡片入口一个都没少认', () => {
    // 入口名改了而这里没跟着改，下面的断言就会形同虚设。
    for (const entry of CARD_ENTRY_POINTS) {
      expect(SOURCE, `卡片入口 ${entry} 在源码里找不到了，先确认它是不是改名了`)
        .toContain(entry)
    }
  })

  it('没有任何一处取用已删除的「没有取得识别结果」文案', () => {
    // 这两个 key 已经从两份 locale 里删掉了，取它只会渲染出 key 本身。
    // 只查真实调用形式 `t('...')`，不查裸标识符 —— 否则注释里提一句名字都会误报。
    expect(SOURCE).not.toContain("t('recorder.noResultTitle')")
    expect(SOURCE).not.toContain("t('recorder.noResultDetail')")
  })

  it('每一处卡片的标题都在白名单里', () => {
    // 逐个卡片入口往后扫一小段，把它传的 title key 抓出来比对。
    const titles: string[] = []
    for (const entry of CARD_ENTRY_POINTS) {
      let from = 0
      for (; ;) {
        const at = SOURCE.indexOf(entry, from)
        if (at < 0) break
        from = at + entry.length
        // 参数对象就在调用后面，标题是第一个 t('recorder.xxxTitle')。
        const window = SOURCE.slice(at, at + 400)
        const matched = /title:\s*t\('([^']+)'\)/.exec(window)
        if (matched) titles.push(matched[1])
      }
    }

    // 抓到 0 条说明正则和源码写法不再匹配 —— 那时这条断言等于没跑。
    expect(titles.length).toBeGreaterThan(0)
    for (const title of titles) {
      expect(CARD_TITLE_KEYS, `卡片标题 ${title} 不在白名单里`).toContain(title)
    }
  })

  it('两条空结果路径都调了 showNoSpeech，且都带 no_text 分支', () => {
    // 漏改的那一次，源码里 showNoSpeech 只出现在一条路径上。
    const noSpeechCalls = SOURCE.match(/showNoSpeech\(/g) ?? []
    expect(noSpeechCalls.length).toBeGreaterThanOrEqual(3)
    const noTextBranches = SOURCE.match(/silenceProven \? 'silent' : 'no_text'/g) ?? []
    expect(noTextBranches.length).toBe(2)
  })
})
