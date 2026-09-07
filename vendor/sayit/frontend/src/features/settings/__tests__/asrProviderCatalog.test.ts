import { describe, it, expect } from 'vitest'
import {
  ASR_PLATFORMS,
  ASR_PROVIDERS,
  asrAvailabilityLabel,
  describeAsrMissing,
  effectiveAsrCredentials,
  emptyAsrProfile,
  findAsrProvider,
  gradeAsrLatency,
  keyFingerprint,
  parseAsrProfiles,
  providersOfPlatform,
  resolveActiveAsrProfile,
  type AsrProfile,
} from '../asrProviderCatalog'

function profile(over: Partial<AsrProfile> = {}): AsrProfile {
  return { ...emptyAsrProfile(), ...over }
}

describe('ASR_PROVIDERS 结构性不变量', () => {
  it('id 唯一', () => {
    const ids = ASR_PROVIDERS.map((p) => p.id)
    expect(new Set(ids).size).toBe(ids.length)
  })

  it('每项都有标题、模型名、一句定位', () => {
    for (const p of ASR_PROVIDERS) {
      expect(p.label.trim()).not.toBe('')
      expect(p.model.trim()).not.toBe('')
      expect(p.blurb.trim()).not.toBe('')
      expect(['mainland_china', 'global']).toContain(p.availability)
    }
  })

  // 这条断言变过两次，记下来免得再绕回去：
  //   v1「每一家都是 mainland_china」—— 只是把当时的事实写死，加海外供应商时它拦的是新功能；
  //   v2「每种地区都有展示文案」—— 于是七张卡上各挂一句「面向中国大陆账号」，是纯噪音；
  //   现在：国内是默认情形，不给字；只有需要海外账号/网络的才提醒。
  it('只有海外供应商才给地区提示，国内的不占字', () => {
    for (const p of ASR_PROVIDERS) {
      const label = asrAvailabilityLabel(p).trim()
      if (p.availability === 'global') expect(label).not.toBe('')
      else expect(label).toBe('')
    }
    // 至少还有一家是海外的，否则上面那半条断言等于没跑
    expect(ASR_PROVIDERS.some((p) => p.availability === 'global')).toBe(true)
  })

  it('平台都在 ASR_PLATFORMS 里有定义', () => {
    for (const p of ASR_PROVIDERS) expect(ASR_PLATFORMS[p.platform]).toBeTruthy()
  })

  // 卡片标题不该再出现「（qwen3.5-omni-plus，ASR+AI）」这种括号串——模型名有自己的一行
  it('标题不带括号与模型名', () => {
    for (const p of ASR_PROVIDERS) {
      expect(p.label).not.toMatch(/[（(]/)
      expect(p.label).not.toContain(p.model)
    }
  })

  it('5 个千问变体归到同一个平台', () => {
    expect(providersOfPlatform('qwen').map((p) => p.id)).toEqual([
      'qwen_audio_stream', 'qwen', 'qwen_realtime', 'qwen_omni_35_plus', 'qwen_omni_35_flash',
    ])
  })

  /** 清单顺序就是推荐顺序（卡片顺序 + 新建下拉的顺序），前两名钉住 */
  it('推荐顺序：豆包第一，千问 Audio 3.0 第二', () => {
    expect(ASR_PROVIDERS.slice(0, 2).map((p) => p.id)).toEqual(['doubao_v2', 'qwen_audio_stream'])
  })

  /**
   * 只有 qwen3-asr-flash-realtime 需要业务空间 ID。
   * qwen-audio-3.0-asr-flash-streaming 用通用域名就能跑（实测），加卡时照抄
   * needsWorkspaceId 会让用户以为不填就用不了 —— 这条钉住这个区别。
   */
  it('只有 qwen_realtime 需要业务空间 ID', () => {
    expect(ASR_PROVIDERS.filter((p) => p.needsWorkspaceId).map((p) => p.id)).toEqual([
      'qwen_realtime',
    ])
  })

  it('findAsrProvider 查得到也查不崩', () => {
    expect(findAsrProvider('doubao_v2')?.platform).toBe('doubao')
    expect(findAsrProvider('nope')).toBeUndefined()
  })
})

describe('parseAsrProfiles 容错', () => {
  it('非数组一律当空列表', () => {
    expect(parseAsrProfiles(null)).toEqual([])
    expect(parseAsrProfiles({})).toEqual([])
    expect(parseAsrProfiles('x')).toEqual([])
  })

  it('丢掉供应商不在内置清单里的条目', () => {
    // 留着只会渲染出一张点不动的卡
    const r = parseAsrProfiles([
      { id: 'a', provider: 'doubao_v2' },
      { id: 'b', provider: 'gpt-4-asr' },
      { id: 'c' },
    ])
    expect(r.map((p) => p.id)).toEqual(['a'])
  })

  it('丢掉重复 id', () => {
    const r = parseAsrProfiles([
      { id: 'a', provider: 'doubao_v2' },
      { id: 'a', provider: 'qwen' },
    ])
    expect(r).toHaveLength(1)
  })

  it('缺字段补成空串，不会渲染出 undefined', () => {
    const [p] = parseAsrProfiles([{ id: 'a', provider: 'qwen' }])
    expect(p.apiKey).toBe('')
    expect(p.appId).toBe('')
    expect(p.workspaceId).toBe('')
    expect(p.omniPrompt).toBe('')
    expect(p.console).toBe('new')
  })

  it('console 只认 new / legacy，其它值回落到 new', () => {
    expect(parseAsrProfiles([{ id: 'a', provider: 'doubao_v2', console: 'legacy' }])[0].console).toBe('legacy')
    expect(parseAsrProfiles([{ id: 'a', provider: 'doubao_v2', console: 'garbage' }])[0].console).toBe('new')
  })

  it('坏的检测结论被丢掉，好的保留', () => {
    const r = parseAsrProfiles([
      { id: 'a', provider: 'qwen', check: { ok: true, at: 100, latencyMs: 800, audioSec: 3 } },
      { id: 'b', provider: 'qwen', check: { ok: true } },
      { id: 'c', provider: 'qwen', check: 'nope' },
    ])
    expect(r[0].check).toEqual({ ok: true, at: 100, latencyMs: 800, audioSec: 3, reason: undefined })
    expect(r[1].check).toBeUndefined()
    expect(r[2].check).toBeUndefined()
  })

  it('没有 id 时自动补一个，不会整条丢掉', () => {
    const r = parseAsrProfiles([{ provider: 'qwen', apiKey: 'sk-1' }])
    expect(r).toHaveLength(1)
    expect(r[0].id).not.toBe('')
  })
})

describe('resolveActiveAsrProfile', () => {
  it('指向不存在的 id 时回落到第一条', () => {
    const list = [profile({ id: 'a' }), profile({ id: 'b' })]
    expect(resolveActiveAsrProfile(list, 'zzz')?.id).toBe('a')
    expect(resolveActiveAsrProfile(list, 'b')?.id).toBe('b')
  })

  it('空列表返回 null', () => {
    expect(resolveActiveAsrProfile([], 'a')).toBeNull()
  })
})

describe('effectiveAsrCredentials', () => {
  // 核心不变量：Rust 侧靠 app_id 空不空来选两代鉴权头
  it('豆包新版控制台必须把 appId 清空', () => {
    const p = profile({
      provider: 'doubao_v2', console: 'new', apiKey: 'APPKEY', otherKey: 'TOKEN', appId: '1234567890',
    })
    expect(effectiveAsrCredentials(p)).toEqual({ apiKey: 'APPKEY', appId: '' })
  })

  it('豆包旧版控制台用 Access Token + App ID', () => {
    const p = profile({
      provider: 'doubao_v2', console: 'legacy', apiKey: 'TOKEN', otherKey: 'APPKEY', appId: '1234567890',
    })
    expect(effectiveAsrCredentials(p)).toEqual({ apiKey: 'TOKEN', appId: '1234567890' })
  })

  it('非豆包平台不带 appId', () => {
    const p = profile({ provider: 'qwen', apiKey: ' sk-1 ', appId: '999' })
    expect(effectiveAsrCredentials(p)).toEqual({ apiKey: 'sk-1', appId: '' })
  })
})

describe('describeAsrMissing', () => {
  it('豆包新版只要密钥', () => {
    expect(describeAsrMissing(profile({ provider: 'doubao_v2', console: 'new' }))).toBe('还没填 API Key')
    expect(describeAsrMissing(profile({ provider: 'doubao_v2', console: 'new', apiKey: 'k' }))).toBe('')
  })

  it('豆包旧版两个都要，且先报密钥', () => {
    expect(describeAsrMissing(profile({ provider: 'doubao_v2', console: 'legacy' }))).toBe('还没填 Access Token')
    expect(describeAsrMissing(profile({ provider: 'doubao_v2', console: 'legacy', apiKey: 't' }))).toBe('还没填 App ID')
    expect(describeAsrMissing(profile({ provider: 'doubao_v2', console: 'legacy', apiKey: 't', appId: '1' }))).toBe('')
  })

  it('其它平台只要 API Key', () => {
    expect(describeAsrMissing(profile({ provider: 'mimo' }))).toBe('还没填 API Key')
    expect(describeAsrMissing(profile({ provider: 'mimo', apiKey: 'k' }))).toBe('')
  })
})

describe('keyFingerprint', () => {
  it('只露最后 4 位，够区分同一家的两份配置', () => {
    expect(keyFingerprint(profile({ provider: 'qwen', apiKey: 'sk-abcdef1234' }))).toBe('••1234')
  })

  it('密钥太短或为空时不给指纹，避免把整把密钥露出来', () => {
    expect(keyFingerprint(profile({ provider: 'qwen', apiKey: 'ab' }))).toBe('')
    expect(keyFingerprint(profile({ provider: 'qwen', apiKey: '' }))).toBe('')
  })

  it('豆包按当前代次的生效密钥取，不会取到另一代那把', () => {
    const p = profile({ provider: 'doubao_v2', console: 'new', apiKey: 'NEWKEY9999', otherKey: 'OLD1111' })
    expect(keyFingerprint(p)).toBe('••9999')
  })
})

describe('gradeAsrLatency', () => {
  // 口径是 RTF，不是绝对毫秒。3 秒音频花 1 秒是正常水平，
  // 若照搬 AI 服务那套「1 秒就算慢」的阈值，所有 ASR 都会被标成太慢。
  it('3 秒音频花 1 秒算正常，不算慢', () => {
    expect(gradeAsrLatency(1000, 3).tone).toBe('ok')
    expect(gradeAsrLatency(1000, 3).label).toBe('正常')
  })

  it('按 RTF 分档', () => {
    expect(gradeAsrLatency(300, 3).label).toBe('极速')
    expect(gradeAsrLatency(700, 3).label).toBe('很快')
    expect(gradeAsrLatency(1200, 3).label).toBe('正常')
    expect(gradeAsrLatency(2000, 3).label).toBe('偏慢')
    expect(gradeAsrLatency(3000, 3).label).toBe('太慢')
  })

  it('同样的毫秒数，音频越长档位越好', () => {
    // 刻意不取边界值：阈值是「小于」，1500/10s 正好等于 0.15 会落进 fast 而非 instant
    expect(gradeAsrLatency(1000, 2).tier).toBe('slow')
    expect(gradeAsrLatency(1000, 10).tier).toBe('instant')
  })

  it('阈值是「小于」，边界值落进下一档', () => {
    expect(gradeAsrLatency(150, 1).tier).toBe('fast')
    expect(gradeAsrLatency(149, 1).tier).toBe('instant')
  })

  it('只有太慢才给 bad —— 红色留给「不可用」', () => {
    expect(gradeAsrLatency(2000, 3).tone).toBe('warn')
    expect(gradeAsrLatency(9000, 3).tone).toBe('bad')
  })

  it('音频时长缺失时不硬猜档位', () => {
    expect(gradeAsrLatency(1000, 0).label).toBe('已测通')
    expect(gradeAsrLatency(1000, NaN).label).toBe('已测通')
  })
})
