import { describe, it, expect, vi, beforeEach } from 'vitest'

// 存储层走 Tauri IPC，换成一个内存 Map。用 vi.hoisted 是因为 vi.mock 的工厂会被提到
// 文件顶部执行，普通 const 那时还没初始化。
const { store } = vi.hoisted(() => ({ store: new Map<string, unknown>() }))
vi.mock('@/services/store', () => ({
  getSetting: (key: string, fallback: unknown) =>
    Promise.resolve(store.has(key) ? store.get(key) : fallback),
  setSetting: (key: string, value: unknown) => {
    store.set(key, value)
    return Promise.resolve()
  },
}))
vi.mock('@/services/debugLog', () => ({ addRuntimeEvent: () => { } }))

import {
  ASR_ACTIVE_PROFILE_KEY,
  ASR_PROFILES_KEY,
  loadAsrProfiles,
  saveAsrProfiles,
  topUpProfiles,
} from '../asrProfileStore'
import {
  asrModelsOf,
  emptyAsrProfile,
  findAsrProvider,
  type AsrProfile,
} from '../asrProviderCatalog'

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
   * 这条规则变过两次，两次的理由都写在 topUpProfiles 的注释里。现在是「每平台一张卡」，
   * 而且这次不会丢东西 —— 该平台的全部模型都在那张卡的下拉里。
   */
  it('一个平台填了密钥，就给这个平台建一张卡', () => {
    const r = topUpProfiles([], { qwen: creds({ apiKey: 'sk-1' }) })
    expect(r.profiles.map((p) => p.provider)).toEqual(['qwen'])
    expect(r.added).toEqual(['qwen'])
    // 那张卡覆盖千问全部模型（原来占 5 张卡的那些，加上两个上一代 Omni），一个都没丢
    expect(asrModelsOf(findAsrProvider('qwen')!).length).toBeGreaterThanOrEqual(5)
  })

  it('三个平台都有密钥时补出三张卡', () => {
    const r = topUpProfiles([], {
      doubao: creds(), qwen: creds({ apiKey: 'sk-1' }), mimo: creds(),
    })
    expect(r.profiles.map((p) => p.provider)).toEqual(['doubao', 'qwen', 'mimo'])
  })

  it('没填密钥的平台不建卡 —— 新用户应看到空状态引导，而不是一堆空卡', () => {
    expect(topUpProfiles([], {}).profiles).toEqual([])
    expect(topUpProfiles([], { qwen: creds({ apiKey: '   ' }) }).profiles).toEqual([])
    expect(topUpProfiles([], { mimo: creds({ apiKey: '', otherKey: '' }) }).profiles).toEqual([])
  })

  it('只补缺的平台，已有档案原样不动（幂等）', () => {
    const existing = [profile('qwen', { apiKey: 'MY-OWN' })]
    const r = topUpProfiles(existing, {
      qwen: creds({ apiKey: 'sk-1' }), doubao: creds(),
    })
    expect(r.profiles).toHaveLength(2)
    expect(r.profiles[0]).toBe(existing[0]) // 同一个对象，没被替换
    expect(r.added).toEqual(['doubao'])
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
  it('已经自动补过又被删掉的卡不再重建', () => {
    const r = topUpProfiles([], { qwen: creds({ apiKey: 'sk-1' }) }, ['qwen'])
    expect(r.added).toEqual([])
  })

  /**
   * 存量的「补过谁」那份记账里存的是**旧的分发 key**（`qwen_audio_stream` 之类）。
   * 不换算成卡片 id 的话新代码认不出来，会把老用户主动删掉的卡在升级后补回来一次。
   */
  it('存量记账里的旧 provider id 也算已补过', () => {
    const r = topUpProfiles([], { qwen: creds({ apiKey: 'sk-1' }) }, ['qwen_audio_stream'])
    expect(r.added).toEqual([])
    const other = topUpProfiles([], { doubao: creds() }, ['doubao_v2'])
    expect(other.added).toEqual([])
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

  /**
   * System Prompt 只给「有 Omni 模型的平台」。
   *
   * 它是 profile 级字段、只在用户真选了 Omni 模型时才生效，所以千问那张卡先备着；
   * 豆包那张卡没有 Omni 模型，带上就是一个永远用不到的字段。
   */
  it('System Prompt 只给有 Omni 模型的平台', () => {
    const { profiles } = topUpProfiles([], {
      qwen: creds({ apiKey: 'sk-1', omniPrompt: 'MY PROMPT' }),
      doubao: creds({ omniPrompt: 'MY PROMPT' }),
    })
    const byId = new Map(profiles.map((p) => [p.provider, p]))
    expect(byId.get('qwen')!.omniPrompt).toBe('MY PROMPT')
    expect(byId.get('doubao')!.omniPrompt).toBe('')
  })

  it('业务空间 ID 跟着千问平台走', () => {
    const { profiles } = topUpProfiles([], { qwen: creds({ apiKey: 'sk-1', workspaceId: 'ws-abc' }) })
    expect(profiles.every((p) => p.workspaceId === 'ws-abc')).toBe(true)
  })

  it('补出来的档案 id 互不相同', () => {
    const { profiles } = topUpProfiles([], { doubao: creds(), qwen: creds(), mimo: creds() })
    expect(new Set(profiles.map((p) => p.id)).size).toBe(profiles.length)
  })

  /**
   * 补出来的档案必须**显式带上默认模型**，不能留空串。
   *
   * parseAsrProfiles 靠「有没有 model」区分新老数据（`qwen`/`mimo`/`doubao` 这几个值
   * 既是旧分发 key 又是新卡片 id，光看 provider 分不出来）。留空会让这些新建的条目
   * 下次加载时被当成存量数据走迁移，于是模型被换成迁移表里那个。
   */
  it('补出来的档案显式写上默认模型', () => {
    const { profiles } = topUpProfiles([], { doubao: creds(), qwen: creds() })
    const byId = new Map(profiles.map((p) => [p.provider, p]))
    expect(byId.get('doubao')!.model).toBe('Doubao-Seed-ASR-2.0')
    expect(byId.get('qwen')!.model).toBe('qwen-audio-3.1-asr-flash-streaming')
    expect(profiles.every((p) => p.model !== '')).toBe(true)
  })
})

/**
 * 落盘往返。
 *
 * 为什么必须单独测这一路：光测 `parseAsrProfilesDetailed` 的返回值不算数 ——
 * 真正把配置从磁盘上抹掉的是随后那次「归一化后写回」。写回时不把 orphans 拼回去的话，
 * 解析侧的断言照样全绿，而用户的密钥又丢一次（已经发生过一次，丢的是 Gemini 与
 * OpenRouter 两张卡）。所以判据钉在**存储里最终有什么**，不是解析函数返回了什么。
 */
describe('loadAsrProfiles / saveAsrProfiles 的落盘往返', () => {
  beforeEach(() => {
    store.clear()
  })

  it('解析不出来的条目在写回之后依然躺在存储里', async () => {
    // 未来版本写的卡片 / 手改坏的条目 —— 现在这份代码认不出来
    const alien = { id: 'p-alien', provider: 'some-future-platform', model: 'x', apiKey: 'SECRET' }
    store.set(ASR_PROFILES_KEY, [
      { id: 'p-1', provider: 'doubao', model: 'Doubao-Seed-ASR-2.0', apiKey: 'TOKEN' },
      alien,
    ])

    // load 自己就会写一次（要落 activeId），这正是当初丢数据的那条路径
    const state = await loadAsrProfiles()
    expect(state.profiles.map((p) => p.provider)).toEqual(['doubao'])
    expect(store.get(ASR_PROFILES_KEY)).toContainEqual(alien)

    // 用户接着在设置页改了点别的，再写一次也不能吃掉它
    await saveAsrProfiles(state)
    const written = store.get(ASR_PROFILES_KEY) as unknown[]
    expect(written).toContainEqual(alien)
    expect(written).toHaveLength(2)
  })

  it('全都认得时不会往存储里塞多余条目', async () => {
    store.set(ASR_PROFILES_KEY, [
      { id: 'p-1', provider: 'doubao', model: 'Doubao-Seed-ASR-2.0', apiKey: 'TOKEN' },
    ])
    const state = await loadAsrProfiles()
    await saveAsrProfiles(state)
    expect(store.get(ASR_PROFILES_KEY)).toHaveLength(1)
    expect(store.get(ASR_ACTIVE_PROFILE_KEY)).toBe('p-1')
  })

  /**
   * 运行时键写的是**选中模型**的分发 key，不是卡片 id。写成卡片 id 的话
   * Rust 侧会报 "ASR provider not implemented"（一张卡里的模型可以走不同协议）。
   */
  it('写回时把选中模型的分发 key 同步进运行时键', async () => {
    store.set(ASR_PROFILES_KEY, [
      { id: 'p-1', provider: 'qwen', model: 'qwen3-asr-flash', apiKey: 'sk-1' },
    ])
    await loadAsrProfiles()
    expect(store.get('cloudAsr.provider')).toBe('qwen')
    expect(store.get('cloudAsr.model')).toBe('qwen3-asr-flash')
    expect(store.get('cloudAsr.apiKey')).toBe('sk-1')
  })
})
