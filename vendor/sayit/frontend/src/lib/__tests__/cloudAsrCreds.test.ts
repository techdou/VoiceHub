import { describe, it, expect } from 'vitest'
import {
  describeDoubaoMissing,
  doubaoKeyLabel,
  effectiveDoubaoCredentials,
  resolveDoubaoConsole,
  type DoubaoCredentials,
} from '../cloudAsrCreds'

function creds(over: Partial<DoubaoCredentials> = {}): DoubaoCredentials {
  return { console: 'new', consoleKey: '', accessToken: '', appId: '', ...over }
}

describe('resolveDoubaoConsole', () => {
  it('存过就用存的值', () => {
    expect(resolveDoubaoConsole('new', '1234567890')).toBe('new')
    expect(resolveDoubaoConsole('legacy', '')).toBe('legacy')
  })

  it('没存过但有 App ID = 老用户，判为旧版，行为不变', () => {
    expect(resolveDoubaoConsole(null, '1234567890')).toBe('legacy')
    expect(resolveDoubaoConsole(undefined, '1234567890')).toBe('legacy')
    expect(resolveDoubaoConsole('', '1234567890')).toBe('legacy')
  })

  it('全新用户默认新版', () => {
    expect(resolveDoubaoConsole(null, '')).toBe('new')
    expect(resolveDoubaoConsole('', '   ')).toBe('new')
  })

  it('存了无法识别的值时按 App ID 兜底，不崩', () => {
    expect(resolveDoubaoConsole('garbage', '123')).toBe('legacy')
    expect(resolveDoubaoConsole('garbage', '')).toBe('new')
  })
})

describe('effectiveDoubaoCredentials', () => {
  // 这条是整个改动的核心不变量：Rust 侧靠 app_id 空不空来选鉴权头。
  it('新版模式必须把 appId 清空，否则会用旧版格式发新版密钥', () => {
    const r = effectiveDoubaoCredentials(
      creds({ console: 'new', consoleKey: 'APPKEY', appId: '1234567890', accessToken: 'TOKEN' }),
    )
    expect(r).toEqual({ apiKey: 'APPKEY', appId: '' })
  })

  it('旧版模式用 Access Token + App ID', () => {
    const r = effectiveDoubaoCredentials(
      creds({ console: 'legacy', consoleKey: 'APPKEY', accessToken: 'TOKEN', appId: '1234567890' }),
    )
    expect(r).toEqual({ apiKey: 'TOKEN', appId: '1234567890' })
  })

  it('两代密钥互不串用', () => {
    const both = creds({ consoleKey: 'NEWKEY', accessToken: 'OLDTOKEN', appId: '123' })
    expect(effectiveDoubaoCredentials({ ...both, console: 'new' }).apiKey).toBe('NEWKEY')
    expect(effectiveDoubaoCredentials({ ...both, console: 'legacy' }).apiKey).toBe('OLDTOKEN')
  })

  it('去掉粘贴时常带进来的前后空白', () => {
    expect(effectiveDoubaoCredentials(creds({ console: 'new', consoleKey: '  K  ' })))
      .toEqual({ apiKey: 'K', appId: '' })
    expect(
      effectiveDoubaoCredentials(
        creds({ console: 'legacy', accessToken: ' T ', appId: ' 123 ' }),
      ),
    ).toEqual({ apiKey: 'T', appId: '123' })
  })
})

describe('describeDoubaoMissing', () => {
  it('新版只需要 API Key', () => {
    expect(describeDoubaoMissing(creds({ console: 'new' }))).toBe('还没填 API Key')
    expect(describeDoubaoMissing(creds({ console: 'new', consoleKey: 'K' }))).toBe('')
  })

  it('新版不因为缺 App ID 而拦人', () => {
    expect(describeDoubaoMissing(creds({ console: 'new', consoleKey: 'K', appId: '' }))).toBe('')
  })

  it('旧版两个都要，且先报密钥', () => {
    expect(describeDoubaoMissing(creds({ console: 'legacy' }))).toBe('还没填 Access Token')
    expect(describeDoubaoMissing(creds({ console: 'legacy', accessToken: 'T' })))
      .toBe('还没填 App ID')
    expect(describeDoubaoMissing(creds({ console: 'legacy', accessToken: 'T', appId: '123' })))
      .toBe('')
  })
})

describe('doubaoKeyLabel', () => {
  it('两代叫法不同，标错用户会在控制台找不到', () => {
    expect(doubaoKeyLabel('new')).toBe('API Key')
    expect(doubaoKeyLabel('legacy')).toBe('Access Token')
  })
})
