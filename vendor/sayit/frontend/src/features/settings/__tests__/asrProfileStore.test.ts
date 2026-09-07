import { describe, it, expect } from 'vitest'
import { topUpProfiles } from '../asrProfileStore'
import { emptyAsrProfile, type AsrProfile } from '../asrProviderCatalog'

function creds(over: Record<string, unknown> = {}) {
  return {
    apiKey: 'KEY', otherKey: '', appId: '', console: 'new' as const,
    workspaceId: '', omniPrompt: 'P', ...over,
  }
}

function profile(provider: string, over: Partial<AsrProfile> = {}): AsrProfile {
  return { ...emptyAsrProfile(provider), ...over }
}

describe('topUpProfiles', () => {
  /**
   * 这条是回归测试。第一版迁移按「每个平台一份」建档案，千问平台下 4 个变体
   * （一次性 / 流式 / 两个 Omni）被压成 1 张卡，用户升级后发现服务少了 3 个。
   */
  it('一个平台填了密钥，就为该平台下每个服务各建一份', () => {
    const r = topUpProfiles([], { qwen: creds({ apiKey: 'sk-1' }) })
    expect(r.profiles.map((p) => p.provider)).toEqual([
      'qwen_audio_stream', 'qwen', 'qwen_realtime', 'qwen_omni_35_plus', 'qwen_omni_35_flash',
    ])
    expect(r.added).toHaveLength(5)
  })

  it('三个平台都有密钥时补出全部 7 个服务', () => {
    const r = topUpProfiles([], {
      doubao: creds(), qwen: creds({ apiKey: 'sk-1' }), mimo: creds(),
    })
    expect(r.profiles).toHaveLength(7)
  })

  it('没填密钥的平台不建卡 —— 新用户应看到空状态引导，而不是一堆空卡', () => {
    expect(topUpProfiles([], {}).profiles).toEqual([])
    expect(topUpProfiles([], { qwen: creds({ apiKey: '   ' }) }).profiles).toEqual([])
    expect(topUpProfiles([], { mimo: creds({ apiKey: '', otherKey: '' }) }).profiles).toEqual([])
  })

  /**
   * 真实场景回归：第一版迁移只建了 doubao_v2 / qwen_realtime / mimo 三份，
   * 补齐必须把千问平台下其余几个加上，且不动已有的那份。
   */
  it('在第一版迁移的结果上补齐千问剩下几个', () => {
    const existing = [profile('doubao_v2'), profile('qwen_realtime'), profile('mimo')]
    const r = topUpProfiles(existing, {
      doubao: creds(), qwen: creds({ apiKey: 'sk-1' }), mimo: creds(),
    })
    expect(r.added).toEqual(['qwen_audio_stream', 'qwen', 'qwen_omni_35_plus', 'qwen_omni_35_flash'])
    expect(r.profiles).toHaveLength(7)
    expect(r.profiles.slice(0, 3)).toEqual(existing) // 原有三份原样不动
  })

  it('只补缺的，已有档案原样不动（对已经自己建过卡的用户幂等）', () => {
    const existing = [profile('qwen', { apiKey: 'MY-OWN' })]
    const r = topUpProfiles(existing, { qwen: creds({ apiKey: 'sk-1' }) })
    expect(r.profiles).toHaveLength(5)
    expect(r.profiles[0]).toBe(existing[0]) // 同一个对象，没被替换
    expect(r.profiles.filter((p) => p.provider === 'qwen')).toHaveLength(1)
  })

  it('重复调用不会越补越多', () => {
    const once = topUpProfiles([], { doubao: creds() })
    const twice = topUpProfiles(once.profiles, { doubao: creds() })
    expect(twice.profiles).toEqual(once.profiles)
    expect(twice.added).toEqual([])
  })

  /**
   * 用户主动删掉的卡不该被"救回来"。
   * 记「补过谁」而不是「补完了」，就是为了同时满足自愈与尊重删除这两件事。
   */
  it('已经自动补过又被删掉的服务不再重建', () => {
    const r = topUpProfiles([], { qwen: creds({ apiKey: 'sk-1' }) }, ['qwen', 'qwen_omni_35_flash'])
    expect(r.added).toEqual(['qwen_audio_stream', 'qwen_realtime', 'qwen_omni_35_plus'])
  })

  it('豆包的控制台代次与两代密钥都带过去', () => {
    const { profiles: [p] } = topUpProfiles([], {
      doubao: creds({ apiKey: 'TOKEN', otherKey: 'APPKEY', appId: '123', console: 'legacy' }),
    })
    expect(p.console).toBe('legacy')
    expect(p.apiKey).toBe('TOKEN')
    expect(p.otherKey).toBe('APPKEY')
    expect(p.appId).toBe('123')
  })

  it('只有豆包侧填了 App ID + Access Token（apiKey 为空）也算有密钥', () => {
    // 旧版控制台用户可能只有 otherKey 那一侧有值，不能因此判成"没配置"
    const r = topUpProfiles([], { doubao: creds({ apiKey: '', otherKey: 'APPKEY' }) })
    expect(r.profiles).toHaveLength(1)
  })

  it('System Prompt 只给 Omni 服务，普通识别服务不带', () => {
    const { profiles } = topUpProfiles([], { qwen: creds({ apiKey: 'sk-1', omniPrompt: 'MY PROMPT' }) })
    const omni = profiles.filter((p) => p.provider.includes('omni'))
    const plain = profiles.filter((p) => !p.provider.includes('omni'))
    expect(omni.every((p) => p.omniPrompt === 'MY PROMPT')).toBe(true)
    expect(plain.every((p) => p.omniPrompt === '')).toBe(true)
  })

  it('业务空间 ID 跟着千问平台走', () => {
    const { profiles } = topUpProfiles([], { qwen: creds({ apiKey: 'sk-1', workspaceId: 'ws-abc' }) })
    expect(profiles.every((p) => p.workspaceId === 'ws-abc')).toBe(true)
  })

  it('补出来的档案 id 互不相同', () => {
    const { profiles } = topUpProfiles([], { doubao: creds(), qwen: creds(), mimo: creds() })
    expect(new Set(profiles.map((p) => p.id)).size).toBe(profiles.length)
  })
})
