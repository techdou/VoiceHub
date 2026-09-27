import { beforeEach, describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({ addRuntimeEvent: vi.fn() }))

vi.mock('../../debugLog', () => ({
  addRuntimeEvent: mocks.addRuntimeEvent,
  AI_LOG_SOURCE: 'ai',
  AI_EVENT_REQUEST: 'ai.request',
  AI_EVENT_OUTCOME: 'ai.outcome',
}))

import {
  __resetAiOutcomeLogDedupe,
  extractServerAiEvidence,
  policyFromSnapshot,
  resolveAiOutcome,
  resolveAiPolicy,
  resolveAndLogAiOutcome,
  serverShouldPolish,
  type AiConfigSnapshot,
} from '../aiPolicy'

const SERVER_MANAGED: AiConfigSnapshot = {
  workMode: 'server',
  aiEnabled: true,
  aiMinDurationSec: 0,
  serverAiSource: 'managed',
}

function policy(over: Partial<Parameters<typeof resolveAiPolicy>[0]> = {}) {
  return resolveAiPolicy({ ...SERVER_MANAGED, audioDurationSec: 10, ...over })
}

describe('resolveAiPolicy：路由与是否允许调用', () => {
  it('服务器 + 内置 AI → managed，允许调用', () => {
    const p = policy()
    expect(p.route).toBe('managed')
    expect(p.allowCall).toBe(true)
    expect(p.reason).toBeUndefined()
  })

  it('服务器 + 自配 AI → custom，仍然允许调用', () => {
    // custom 只表示"服务端不做整理"，客户端会接着调。它永远不是跳过原因。
    const p = policy({ serverAiSource: 'custom' })
    expect(p.route).toBe('custom')
    expect(p.allowCall).toBe(true)
    expect(p.reason).toBeUndefined()
  })

  it.each(['local', 'cloud_api'] as const)('%s 模式一律 custom 路由', (workMode) => {
    // 这两种模式没有服务端 AI，serverAiSource 是残留值也不该影响路由
    expect(policy({ workMode, serverAiSource: 'managed' }).route).toBe('custom')
  })

  it('总开关关闭 → route=none / reason=ai_off', () => {
    const p = policy({ aiEnabled: false, aiMinDurationSec: 60, audioDurationSec: 4.8 })
    expect(p.route).toBe('none')
    expect(p.allowCall).toBe(false)
    // 同时低于门槛时也要报总开关：让用户去调一个根本不生效的门槛是误导
    expect(p.reason).toBe('ai_off')
  })

  it('低于门槛 → reason=duration_below_min，但路由仍保留', () => {
    const p = policy({ aiMinDurationSec: 60, audioDurationSec: 4.8, serverAiSource: 'custom' })
    expect(p.allowCall).toBe(false)
    expect(p.reason).toBe('duration_below_min')
    expect(p.route).toBe('custom')
    expect(p.audioMs).toBe(4800)
    expect(p.minAudioMs).toBe(60000)
  })

  it('门槛为 0 表示不设限', () => {
    expect(policy({ aiMinDurationSec: 0, audioDurationSec: 0.2 }).allowCall).toBe(true)
  })

  it('时长恰好等于门槛时允许调用', () => {
    // 判据是 <，不是 <=。用毫秒整数比较，避免浮点误差把这一档判反。
    expect(policy({ aiMinDurationSec: 2, audioDurationSec: 2 }).allowCall).toBe(true)
    expect(policy({ aiMinDurationSec: 2, audioDurationSec: 1.999 }).allowCall).toBe(false)
  })

  it('识别引擎自带整理 → reason=integrated_asr，不冒充用户预设已执行', () => {
    const p = policy({ workMode: 'cloud_api', integratedAsr: true })
    expect(p.route).toBe('integrated_asr')
    expect(p.allowCall).toBe(false)
    expect(p.reason).toBe('integrated_asr')
  })
})

describe('serverShouldPolish：压成线上那个布尔', () => {
  it('只有 managed 且允许调用时才让服务端整理', () => {
    expect(serverShouldPolish(policy())).toBe(true)
    expect(serverShouldPolish(policy({ serverAiSource: 'custom' }))).toBe(false)
    expect(serverShouldPolish(policy({ aiEnabled: false }))).toBe(false)
    expect(serverShouldPolish(policy({ aiMinDurationSec: 60, audioDurationSec: 1 }))).toBe(false)
  })
})

describe('resolveAiOutcome：结果与原因', () => {
  it('空识别优先于一切：没文本时答案是 empty_asr', () => {
    const o = resolveAiOutcome(policy({ aiMinDurationSec: 60, audioDurationSec: 1 }), { asrTextEmpty: true })
    expect(o).toMatchObject({ source: 'none', status: 'skipped', reason: 'empty_asr', attempted: false })
  })

  it('策略不允许调用 → 原因直接沿用 policy 的，且 attempted=false', () => {
    const o = resolveAiOutcome(policy({ aiMinDurationSec: 60, audioDurationSec: 4.8 }))
    expect(o).toMatchObject({ status: 'skipped', reason: 'duration_below_min', attempted: false, llmMs: 0 })
  })

  it('managed：服务端回了 error → failed，不是 unavailable', () => {
    // 这是本轮修的核心错判：服务端异常时返回原文 + llm_ms=0，
    // 旧代码按耗时判断，把"调用失败"记成了"不可用"。
    const o = resolveAiOutcome(policy(), { serverError: 'HTTP 400 temperature', llmMs: 0 })
    expect(o).toMatchObject({ source: 'server', status: 'failed', reason: 'call_failed', attempted: true })
  })

  it('managed：有 provider 证据即算执行过，哪怕耗时为 0', () => {
    const o = resolveAiOutcome(policy(), { serverProvider: 'openai', llmMs: 0 })
    expect(o).toMatchObject({ status: 'applied', attempted: true, provider: 'server' })
  })

  it('managed：只有耗时 > 0 也算执行过（兼容不带 provider 的服务器）', () => {
    expect(resolveAiOutcome(policy(), { llmMs: 1228 }).status).toBe('applied')
  })

  it('managed：既无 provider 也无耗时 → no_evidence，不得断言 failed', () => {
    const o = resolveAiOutcome(policy(), { llmMs: 0 })
    expect(o).toMatchObject({ status: 'unavailable', reason: 'no_evidence', attempted: false })
    expect(o.status).not.toBe('failed')
  })

  it('custom：配置不完整 → unavailable / config_incomplete，未尝试', () => {
    const o = resolveAiOutcome(policy({ serverAiSource: 'custom' }), {
      configComplete: false,
      provider: 'zhipu',
    })
    expect(o).toMatchObject({
      source: 'custom', status: 'unavailable', reason: 'config_incomplete', attempted: false,
    })
  })

  it('custom：超时与调用失败分开记', () => {
    const p = policy({ serverAiSource: 'custom' })
    expect(resolveAiOutcome(p, { clientFailure: 'timeout' }).reason).toBe('call_timeout')
    expect(resolveAiOutcome(p, { clientFailure: 'error' }).reason).toBe('call_failed')
    expect(resolveAiOutcome(p, { clientFailure: 'error' }).attempted).toBe(true)
  })

  it('custom：返回空文本 → failed / empty_output', () => {
    const o = resolveAiOutcome(policy({ serverAiSource: 'custom' }), { clientOutputEmpty: true, llmMs: 90 })
    expect(o).toMatchObject({ status: 'failed', reason: 'empty_output', attempted: true, llmMs: 90 })
  })

  it('custom：成功 → applied，并带上供应商与模型', () => {
    const o = resolveAiOutcome(policy({ serverAiSource: 'custom' }), {
      llmMs: 680, provider: 'zhipu', model: 'GLM-5.3-Flash',
    })
    expect(o).toMatchObject({
      source: 'custom', status: 'applied', attempted: true, provider: 'zhipu', model: 'GLM-5.3-Flash',
    })
    expect(o.reason).toBeUndefined()
  })

  it('不拿文本相等当判据：整理成功但文本没变化仍算成功', () => {
    // 判据里压根没有文本参数，这条钉的是"以后也不许加进来"
    const o = resolveAiOutcome(policy(), { serverProvider: 'openai', llmMs: 5 })
    expect(o.status).toBe('applied')
  })
})

describe('policyFromSnapshot：缓冲型 Provider 复原策略', () => {
  it('快照存在时按快照判，不读当前设置', () => {
    const p = policyFromSnapshot(
      { workMode: 'server', aiEnabled: true, aiMinDurationSec: 60, serverAiSource: 'custom' },
      'local',
      4.8,
    )
    expect(p).toMatchObject({ route: 'custom', allowCall: false, reason: 'duration_below_min' })
    expect(p.workMode).toBe('server')
  })

  it('快照缺失时兜底为"开启、无门槛"，而不是静默不整理', () => {
    const p = policyFromSnapshot(undefined, 'cloud_api', 1)
    expect(p.allowCall).toBe(true)
    expect(p.route).toBe('custom')
  })
})

describe('extractServerAiEvidence：只摘结论，不带调试正文', () => {
  it('摘出 error 与 provider', () => {
    expect(extractServerAiEvidence({ error: 'boom', provider: 'openai' }))
      .toEqual({ error: 'boom', provider: 'openai' })
  })

  it('丢弃 messages / raw_output 等可能含 prompt 与正文的字段', () => {
    const evidence = extractServerAiEvidence({
      provider: 'openai',
      model: 'gpt-x',
      messages: [{ role: 'system', content: '用户的预设正文' }],
      raw_output: '整理后的正文',
    })
    expect(evidence).toEqual({ provider: 'openai' })
    expect(JSON.stringify(evidence)).not.toContain('预设')
    expect(JSON.stringify(evidence)).not.toContain('整理后')
  })

  it('空对象 / 非对象 → undefined', () => {
    expect(extractServerAiEvidence({})).toBeUndefined()
    expect(extractServerAiEvidence(undefined)).toBeUndefined()
    expect(extractServerAiEvidence('nope')).toBeUndefined()
  })
})

describe('resolveAndLogAiOutcome：每次处理恰好一条，且不含正文', () => {
  beforeEach(() => {
    mocks.addRuntimeEvent.mockClear()
    __resetAiOutcomeLogDedupe()
  })

  const ctx = { operationId: 'op-1', trigger: 'live' as const, serverRef: 'cid-abc' }

  it('落一条 ai.outcome，字段齐全且 route 与 reason 并存', () => {
    resolveAndLogAiOutcome(
      ctx,
      policy({ serverAiSource: 'custom', aiMinDurationSec: 60, audioDurationSec: 4.8 }),
    )
    expect(mocks.addRuntimeEvent).toHaveBeenCalledTimes(1)
    const [level, source, message, detail] = mocks.addRuntimeEvent.mock.calls[0]
    expect(level).toBe('info')
    expect(source).toBe('ai')
    expect(message).toBe('ai.outcome')
    expect(detail).toMatchObject({
      operationId: 'op-1',
      trigger: 'live',
      serverRef: 'cid-abc',
      workMode: 'server',
      aiEnabled: true,
      route: 'custom',
      status: 'skipped',
      reason: 'duration_below_min',
      attempted: false,
      audioMs: 4800,
      minAudioMs: 60000,
    })
  })

  it('同一个 operationId 第二次调用不再落盘（asr/final/done 各写一次的结构性防护）', () => {
    resolveAndLogAiOutcome(ctx, policy())
    const second = resolveAndLogAiOutcome(ctx, policy())
    expect(mocks.addRuntimeEvent).toHaveBeenCalledTimes(1)
    // 但返回值仍然正确，调用方不必关心去重
    expect(second.status).toBeDefined()
  })

  it('不同 operationId 各记一条', () => {
    resolveAndLogAiOutcome({ ...ctx, operationId: 'a' }, policy())
    resolveAndLogAiOutcome({ ...ctx, operationId: 'b' }, policy())
    expect(mocks.addRuntimeEvent).toHaveBeenCalledTimes(2)
  })

  it('日志里不出现任何识别或整理正文字段', () => {
    resolveAndLogAiOutcome(ctx, policy(), { serverProvider: 'openai', llmMs: 30 })
    const detail = mocks.addRuntimeEvent.mock.calls[0][3] as Record<string, unknown>
    for (const forbidden of ['asrText', 'llmText', 'text', 'messages', 'systemPrompt', 'raw_output']) {
      expect(Object.keys(detail)).not.toContain(forbidden)
    }
  })
})
