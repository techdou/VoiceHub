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
  it('requires an HTTP endpoint and a model while allowing keyless local services', () => {
    const profile = { ...emptyAsrProfile('openai_compat'), apiUrl: 'http://localhost:8000', model: 'my-asr' }
    expect(describeAsrMissing(profile)).toBe('')
    expect(describeAsrMissing({ ...profile, model: '' })).not.toBe('')
    expect(describeAsrMissing({ ...profile, apiUrl: 'file:///model' })).not.toBe('')
    expect(describeAsrMissing({ ...profile, apiUrl: 'http://user:secret@example.org' })).not.toBe('')
  })
})
