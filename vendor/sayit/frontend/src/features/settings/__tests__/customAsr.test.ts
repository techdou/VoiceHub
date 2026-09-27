import { describe, it, expect } from 'vitest'
import { emptyAsrProfile, parseAsrProfiles, describeAsrMissing } from '../asrProviderCatalog'

describe('custom ASR profiles', () => {
  it('preserves endpoints and model names in saved profiles', () => {
    const profile = { ...emptyAsrProfile('openai_compat'), apiUrl: 'http://localhost:8000/v1', model: 'my-asr' }
    const [loaded] = parseAsrProfiles(JSON.parse(JSON.stringify([profile])))
    expect(loaded.apiUrl).toBe(profile.apiUrl)
    expect(loaded.model).toBe(profile.model)
    expect(describeAsrMissing(loaded)).toBe('')
  })
  it('requires a non-empty endpoint while allowing keyless local services', () => {
    const profile = { ...emptyAsrProfile('openai_compat'), apiUrl: 'http://localhost:8000', model: 'my-asr' }
    expect(describeAsrMissing(profile)).toBe('')
    // 0.2.2 起 model 为空由 store 层统一回落默认（whisper-1），不再算"缺失"；
    // 地址格式（file:// / 带凭据 URL）校验也被上游移除，只拦空地址。
    expect(describeAsrMissing({ ...profile, model: '' })).toBe('')
    expect(describeAsrMissing({ ...profile, apiUrl: '' })).not.toBe('')
    expect(describeAsrMissing({ ...profile, apiUrl: '   ' })).not.toBe('')
  })
})
