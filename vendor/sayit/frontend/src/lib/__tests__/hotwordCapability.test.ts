import { describe, expect, it } from 'vitest'

// Rust 源码当字符串读进来（Vite 的 `?raw`）。用它而不是 node:fs 是因为本项目没装
// @types/node，而 `vite/client` 已经声明了 `*?raw`，不需要额外的类型声明。
// （VoiceHub vendor：native 源码在 vendor/sayit/native/，不在上游的 src-tauri/。）
import capabilitiesSource from '../../../../native/src/providers/capabilities.rs?raw'

import {
  expectedClientCap,
  expectedHotwordDelivery,
  foldHotwordDelivery,
  hotwordDependsOnStreamingPath,
  hotwordUndecidedReason,
  willUseStreamingPath,
  type AsrHotwordCapability,
  type HotwordDelivery,
} from '../asrModels'
import { ASR_PROVIDERS, asrModelsOf } from '@/features/settings/asrProviderCatalog'

/**
 * issue #67 的回归测试。
 *
 * 被修的缺陷是：界面声称热词有效，而 6 家云服务的实现根本没把热词发出去 —— 因为
 * 「哪些家会用热词」的声明在前端一张手写表格里，实现在 Rust 各 provider 里，两者
 * 无人对账。所以这里断言的不是"某一家支不支持"（那种断言在出 bug 的版本上也能写对），
 * 而是**折叠逻辑不会把缺失的声明说成确定的答案**，以及**路径差异被表达了出来**。
 */

function capability(over: Partial<AsrHotwordCapability> = {}): AsrHotwordCapability {
  return {
    streaming: 'context',
    buffered: 'context',
    hasStreamingPath: false,
    streamingClientCap: null,
    bufferedClientCap: null,
    ...over,
  }
}

describe('foldHotwordDelivery', () => {
  it('三种会进请求的传递方式都折成「已发送」', () => {
    for (const delivery of ['vocabulary', 'context', 'instruction'] as HotwordDelivery[]) {
      expect(foldHotwordDelivery(delivery)).toBe('sent')
    }
  })

  it('协议没位置和我们没接，对用户都是「不发送」', () => {
    expect(foldHotwordDelivery('protocol_has_no_slot')).toBe('not_sent')
    expect(foldHotwordDelivery('not_wired_up')).toBe('not_sent')
  })

  /**
   * 这条是整个文件里最要紧的一条。
   *
   * `unknown_provider` 表示 Rust 侧没有声明这个 provider —— 那是个 bug 信号。
   * 一旦把它折成 `not_sent`，界面就会平静地告诉用户"这家不支持热词"，
   * **用一个确定的错误答案盖住一个 bug**，而这正是 issue #67 能藏那么久的机制。
   */
  it('声明缺失绝不能显示成「确定不发送」', () => {
    expect(foldHotwordDelivery('unknown_provider')).toBe('undecided')
    expect(foldHotwordDelivery('unknown_provider')).not.toBe('not_sent')
    expect(foldHotwordDelivery('unknown_provider')).not.toBe('sent')
  })

  it('auto 协议在探测出来之前也是「未确定」，不能猜', () => {
    expect(foldHotwordDelivery('undecided_protocol')).toBe('undecided')
  })
})

describe('expectedHotwordDelivery', () => {
  it('没有流式实现的服务只看一次性路径', () => {
    const cap = capability({ buffered: 'not_wired_up', streaming: 'not_wired_up' })
    expect(expectedHotwordDelivery(cap, {
      streamingDisplayEnabled: true,
      provider: 'groq_whisper',
    })).toBe('not_wired_up')
  })

  /**
   * OpenAI live 的真实形状：流式走 Realtime 的 keywords，关掉字幕回落到文件端点后
   * 一条都不传。这两个答案必须跟着开关走，否则必有一半的用户被告知错误结论。
   */
  it('OpenAI live：开着字幕发送，关掉字幕不发送', () => {
    const cap = capability({
      streaming: 'vocabulary',
      buffered: 'not_wired_up',
      hasStreamingPath: true,
      streamingClientCap: 100,
      bufferedClientCap: null,
    })
    expect(expectedHotwordDelivery(cap, {
      streamingDisplayEnabled: true,
      provider: 'openai_live_transcribe',
    })).toBe('vocabulary')
    expect(expectedHotwordDelivery(cap, {
      streamingDisplayEnabled: false,
      provider: 'openai_live_transcribe',
    })).toBe('not_wired_up')
  })

  /** Gemini live 与上面**方向相反** —— 这是"不能按 provider 一概而论"的现场证据。 */
  it('Gemini live：开着字幕不发送，关掉字幕反而发送', () => {
    const cap = capability({
      streaming: 'protocol_has_no_slot',
      buffered: 'instruction',
      hasStreamingPath: true,
    })
    expect(expectedHotwordDelivery(cap, {
      streamingDisplayEnabled: true,
      provider: 'gemini_live_transcribe',
    })).toBe('protocol_has_no_slot')
    expect(expectedHotwordDelivery(cap, {
      streamingDisplayEnabled: false,
      provider: 'gemini_live_transcribe',
    })).toBe('instruction')
  })

  /**
   * 开关打开不等于会走流式：千问 realtime 缺业务空间 ID 时压根连不上，会回落。
   * 判据必须复用 isStreamingDisplayReady（运行时唯一的那道闸门），
   * 不能只看 streamingDisplayEnabled。
   */
  it('千问 realtime 缺业务空间 ID 时按回落路径算', () => {
    // 千问 realtime 两条路径本来同档（都是 context），看不出闸门有没有生效，
    // 所以这里刻意用一个两档不同的能力去验：判据必须是 isStreamingDisplayReady，
    // 而不是光看 streamingDisplayEnabled。
    const inverted = capability({
      streaming: 'vocabulary',
      buffered: 'not_wired_up',
      hasStreamingPath: true,
    })
    expect(expectedHotwordDelivery(inverted, {
      streamingDisplayEnabled: true,
      provider: 'qwen_realtime',
      qwenWorkspaceId: '',
    })).toBe('not_wired_up')
    expect(expectedHotwordDelivery(inverted, {
      streamingDisplayEnabled: true,
      provider: 'qwen_realtime',
      qwenWorkspaceId: 'ws-123',
    })).toBe('vocabulary')
  })
})

describe('hotwordDependsOnStreamingPath', () => {
  it('两条路径结论相同时不提醒（提醒了是噪音）', () => {
    expect(hotwordDependsOnStreamingPath(capability({
      streaming: 'context',
      buffered: 'context',
      hasStreamingPath: true,
    }))).toBe(false)
    // 传递方式不同但对用户都是"已发送"，也不该提醒
    expect(hotwordDependsOnStreamingPath(capability({
      streaming: 'vocabulary',
      buffered: 'instruction',
      hasStreamingPath: true,
    }))).toBe(false)
  })

  it('结论会被回落翻转时必须提醒', () => {
    expect(hotwordDependsOnStreamingPath(capability({
      streaming: 'vocabulary',
      buffered: 'not_wired_up',
      hasStreamingPath: true,
    }))).toBe(true)
    expect(hotwordDependsOnStreamingPath(capability({
      streaming: 'protocol_has_no_slot',
      buffered: 'instruction',
      hasStreamingPath: true,
    }))).toBe(true)
  })

  it('没有流式路径的服务不会有这个不确定性', () => {
    expect(hotwordDependsOnStreamingPath(capability({
      streaming: 'not_wired_up',
      buffered: 'not_wired_up',
      hasStreamingPath: false,
    }))).toBe(false)
  })
})

/**
 * 从 Rust 源码里读出 `capabilities::ALL_ASR_PROVIDERS` 的真实内容。
 *
 * 这里刻意**不手抄一份**。手抄的问题不是"可能抄错"，而是抄本和真实声明之间没有任何
 * 关系：Rust 那边加一个 key、删一个 key，前端这份副本不动，断言照样全绿 —— 测出来的
 * 只是"我抄得一致吗"。读源码之后，两边的差异必然显形。
 *
 * 解析失败时**抛错而不是返回空集**：返回空集会让下面的子集断言全部平凡通过
 * （空集包含一切），那正是"测试变绿而什么都没验"的经典形状。
 */
function parseAllAsrProviders(rustSource: string): string[] {
  const block = /ALL_ASR_PROVIDERS:\s*&\[&str\]\s*=\s*&\[([\s\S]*?)\];/.exec(rustSource)
  if (!block) {
    throw new Error(
      '在 capabilities.rs 里找不到 ALL_ASR_PROVIDERS 的定义 —— 常量被改名或换了写法，'
      + '这条跨语言断言已经失效，必须同步改这里的解析',
    )
  }
  const keys = [...block[1].matchAll(/"([^"]+)"/g)].map((match) => match[1])
  if (keys.length === 0) {
    throw new Error('ALL_ASR_PROVIDERS 解析出 0 个 key，解析逻辑已失效')
  }
  return keys
}

function declaredProvidersInRust(): Set<string> {
  return new Set(parseAllAsrProviders(capabilitiesSource))
}

/**
 * 一段人造的 Rust 声明，用来验证**解析器本身**对增删敏感。
 *
 * 为什么要这一步：读真实源码只保证"读到的是真的"，不保证"少一个 key 会被看见"。
 * 而那正是这条跨语言断言的全部意义 —— 解析器若在某种写法下静默漏读，
 * 下面的子集断言会平凡通过，漂移照旧发生。
 */
const FAKE_DECLARATION = `
pub const ALL_ASR_PROVIDERS: &[&str] = &[
    "alpha",
    "beta",
    "gamma",
];
`

/**
 * 跨语言的防漂移断言。
 *
 * 前端目录里加一张卡 / 一个模型，它的 `provider` 必须在 Rust 的
 * `capabilities::ALL_ASR_PROVIDERS` 里有声明；漏了的话 Rust 会返回
 * `unknown_provider`，界面显示"未确定"—— 不会骗人，但用户也拿不到答案。
 *
 * 断言方向是**子集**而不是相等：Rust 那份还含前端目录里选不到的内部 key
 * （`openai_chat_audio` 是探测后才分发的）和存量旧 key（`doubao` / `aliyun`）。
 * 「Rust 声明 ↔ 真实分发」那一侧的相等断言在 Rust 里（`capabilities.rs` 的
 * `the_declaration_list_matches_the_dispatch_table`，对照扫的是 `cloud_transcribe` 源码）。
 */
describe('目录里每个模型的运行时 provider 都在 Rust 声明过热词行为', () => {
  const DECLARED_IN_RUST = declaredProvidersInRust()

  it('读到的声明清单是真实的、非空的', () => {
    // 解析器活着的信号。真实清单有十几个 key，读到个位数只可能是解析失效。
    expect(DECLARED_IN_RUST.size).toBeGreaterThanOrEqual(10)
    // 锚点：这个 key 只存在于 Rust 的清单里（前端目录选不到它），
    // 所以它在场就说明读的确实是 Rust 那份，不是哪个前端常量。
    expect(DECLARED_IN_RUST.has('openai_chat_audio')).toBe(true)
  })

  it('解析器认得 Rust 的清单写法', () => {
    expect(parseAllAsrProviders(FAKE_DECLARATION)).toEqual(['alpha', 'beta', 'gamma'])
  })

  /**
   * **从 Rust 清单里删掉一个 key，这边必须立刻少一个。**
   *
   * 这是审核指出的那个洞在前端这一侧：原来 `DECLARED_IN_RUST` 是手抄的，Rust 删一个
   * key 而副本不动，断言照样绿。现在对照来自源码，删除是可见的。
   */
  it('Rust 清单少一个 key，这边就少一个', () => {
    const shrunk = FAKE_DECLARATION.replace('    "beta",\n', '')
    expect(shrunk).not.toBe(FAKE_DECLARATION)
    expect(parseAllAsrProviders(shrunk)).toEqual(['alpha', 'gamma'])
  })

  /** 常量被改名或换写法时必须报错，不能静默返回空集（空集会让子集断言平凡通过）。 */
  it('解析不到时抛错而不是返回空集', () => {
    expect(() => parseAllAsrProviders('pub const SOMETHING_ELSE: &[&str] = &["x"];')).toThrow()
    expect(() => parseAllAsrProviders('pub const ALL_ASR_PROVIDERS: &[&str] = &[];')).toThrow()
  })

  it('遍历的是模型上的 provider，不是卡片 id', () => {
    // 卡片 id 是平台（openai / google / openai_compat），真正调 Rust 的是模型的
    // provider（openai_transcribe / openai_live_transcribe…）。按卡片 id 检查会
    // 漏掉"同一张卡里两个模型走不同实现"这件事，而那正是 OpenAI 与 Gemini 两张卡
    // 的形状 —— 它们卡内就有热词行为相反的两个模型。
    const cardIds = new Set(ASR_PROVIDERS.map((entry) => entry.id))
    const modelProviders = new Set(
      ASR_PROVIDERS.flatMap((entry) => asrModelsOf(entry).map((model) => model.provider)),
    )
    expect(modelProviders.has('openai_transcribe')).toBe(true)
    expect(modelProviders.has('openai_live_transcribe')).toBe(true)
    expect(cardIds.has('openai_transcribe')).toBe(false)
  })

  it('每个模型的 provider 都有声明', () => {
    for (const entry of ASR_PROVIDERS) {
      for (const model of asrModelsOf(entry)) {
        expect(
          DECLARED_IN_RUST.has(model.provider),
          `${entry.id} / ${model.id} 的 provider "${model.provider}" 在 Rust 的热词声明清单里找不到`,
        ).toBe(true)
      }
    }
  })
})

/**
 * 条数上限必须跟着**执行路径**走。
 *
 * 复核时发现的缺陷：上限原来是一个字段，按流式路径算出来就一直用着。于是
 * OpenAI Live 配了 100 个以上热词、再关掉实时字幕，界面会同时说出
 * 「当前识别服务不使用热词」和「最多发送 100 个，其余不会发出」—— 两句话互相矛盾，
 * 而用户没法判断哪句是真的。
 */
describe('expectedClientCap', () => {
  const openaiLive = capability({
    streaming: 'vocabulary',
    buffered: 'not_wired_up',
    hasStreamingPath: true,
    streamingClientCap: 100,
    bufferedClientCap: null,
  })

  it('OpenAI live：开着字幕有上限，关掉字幕不该报上限', () => {
    expect(expectedClientCap(openaiLive, {
      streamingDisplayEnabled: true,
      provider: 'openai_live_transcribe',
    })).toBe(100)
    expect(expectedClientCap(openaiLive, {
      streamingDisplayEnabled: false,
      provider: 'openai_live_transcribe',
    })).toBeNull()
  })

  /** Gemini live 方向相反：上限只存在于关掉字幕后的文件转写路径上。 */
  it('Gemini live：开着字幕没有上限，关掉字幕才有', () => {
    const geminiLive = capability({
      streaming: 'protocol_has_no_slot',
      buffered: 'instruction',
      hasStreamingPath: true,
      streamingClientCap: null,
      bufferedClientCap: 100,
    })
    expect(expectedClientCap(geminiLive, {
      streamingDisplayEnabled: true,
      provider: 'gemini_live_transcribe',
    })).toBeNull()
    expect(expectedClientCap(geminiLive, {
      streamingDisplayEnabled: false,
      provider: 'gemini_live_transcribe',
    })).toBe(100)
  })

  /** 上限和 delivery 必须由同一个路径判断得出，否则就会出现"不发送 + 有上限"。 */
  it('上限和 delivery 永远来自同一条路径', () => {
    for (const streamingDisplayEnabled of [true, false]) {
      const opts = { streamingDisplayEnabled, provider: 'openai_live_transcribe' }
      const delivery = expectedHotwordDelivery(openaiLive, opts)
      const cap = expectedClientCap(openaiLive, opts)
      // 不进请求的路径上不能报上限 —— 这是那句自相矛盾文案的判据
      if (delivery === 'not_wired_up' || delivery === 'protocol_has_no_slot') {
        expect(cap).toBeNull()
      }
      expect(willUseStreamingPath(openaiLive, opts)).toBe(streamingDisplayEnabled)
    }
  })

  it('没有流式实现的服务永远取一次性路径那份', () => {
    const gemini = capability({
      streaming: 'instruction',
      buffered: 'instruction',
      hasStreamingPath: false,
      streamingClientCap: 100,
      bufferedClientCap: 100,
    })
    // 开关开着也不走流式（这家没有流式实现）
    expect(willUseStreamingPath(gemini, {
      streamingDisplayEnabled: true,
      provider: 'gemini_transcribe',
    })).toBe(false)
    expect(expectedClientCap(gemini, {
      streamingDisplayEnabled: true,
      provider: 'gemini_transcribe',
    })).toBe(100)
  })
})

/**
 * 「未确定」的三种成因要能分开。
 *
 * 展示状态共用是对的，但说明不能共用：原来三种都套「等第一次识别或测试连接之后就能
 * 确定」，而那句只对 auto 协议成立。声明缺失是 SayIt 的 bug、查询失败是命令没调通，
 * 这两种重复转写永远不会变 —— 那句话会让用户一直等一个不会来的结果。
 */
describe('hotwordUndecidedReason', () => {
  it('auto 协议未探测是唯一能靠"再用一次"解决的那种', () => {
    expect(hotwordUndecidedReason('undecided_protocol')).toBe('protocol')
  })

  it('声明缺失要单独指认，不能说成"等下次识别"', () => {
    expect(hotwordUndecidedReason('unknown_provider')).toBe('declaration_missing')
    expect(hotwordUndecidedReason('unknown_provider')).not.toBe('protocol')
  })

  it('确定的那些档位没有未确定原因', () => {
    for (const delivery of [
      'vocabulary', 'context', 'instruction', 'not_wired_up', 'protocol_has_no_slot',
    ] as HotwordDelivery[]) {
      expect(hotwordUndecidedReason(delivery)).toBeNull()
    }
  })

  /** 两个函数对同一批档位的判断必须自洽：折成 undecided 的必须给得出原因。 */
  it('凡是折成「未确定」的档位都能给出原因', () => {
    const all: HotwordDelivery[] = [
      'vocabulary', 'context', 'instruction',
      'protocol_has_no_slot', 'not_wired_up',
      'undecided_protocol', 'unknown_provider',
    ]
    for (const delivery of all) {
      const isUndecided = foldHotwordDelivery(delivery) === 'undecided'
      expect(hotwordUndecidedReason(delivery) !== null).toBe(isUndecided)
    }
  })
})
