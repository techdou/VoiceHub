import { describe, expect, it } from 'vitest'
import { normalizeRemoteNotice } from '../notice'

const base = { id: 'notice-1', level: 'info' as const }

describe('normalizeRemoteNotice', () => {
  it('keeps legacy single-language notices compatible', () => {
    const result = normalizeRemoteNotice({ ...base, title: '旧公告', body: '旧正文' }, 'en')
    expect(result?.title).toBe('旧公告')
    expect(result?.body).toBe('旧正文')
  })

  it('selects title, body, and link label for the active locale', () => {
    const payload = {
      ...base,
      title: '维护通知',
      body: '今晚维护',
      linkLabel: '查看详情',
      translations: {
        en: {
          title: 'Maintenance notice',
          body: 'Maintenance tonight',
          linkLabel: 'Learn more',
        },
      },
    }
    expect(normalizeRemoteNotice(payload, 'en')).toMatchObject({
      title: 'Maintenance notice',
      body: 'Maintenance tonight',
      linkLabel: 'Learn more',
    })
    expect(normalizeRemoteNotice(payload, 'zh-CN')).toMatchObject({
      title: '维护通知',
      body: '今晚维护',
      linkLabel: '查看详情',
    })
  })

  it('falls back field by field to legacy strings', () => {
    const result = normalizeRemoteNotice({
      ...base,
      title: 'Fallback title',
      body: 'Fallback body',
      translations: { en: { title: 'English title' } },
    }, 'en')
    expect(result?.title).toBe('English title')
    expect(result?.body).toBe('Fallback body')
  })

  it('rejects payloads without any usable title', () => {
    expect(normalizeRemoteNotice({ ...base, title: '', translations: { en: { title: 'English' } } }, 'en')).toBeNull()
  })
})
