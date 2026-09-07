// 本次版本更新亮点（关于页面展示）。
// 每次发版时更新 version 与 items，保持与 CHANGELOG 同步。
// version 需与打包版本一致，关于页面仅在与当前版本匹配时展示，避免串版。
//
// 写法约定：
// · 按**重要性**排序：先「能力变强」，再「不再被坑」，最后「更顺手」；
// · 一条一句话。语气平实、偏书面，但不生硬：
//     - 别用宣传腔（「任你挑」「一头雾水」这类）；
//     - 也别太口语（「快了一大截」「自己跳一下」「测通不通、快不快」这类）；
//     - 校准点：像产品发布说明，不像聊天记录，也不像广告文案。
// · 说用户看得见的变化，不写模块名、字段名、「重构」这类内部词；
// · 短而不省：一眼能读完，但要让人知道"这对我意味着什么"；
// · 条数压在 10 条以内 —— 列太长等于没重点。

import { t } from '@/i18n'

export interface ReleaseHighlights {
  version: string
  items: string[]
}

export const RELEASE_HIGHLIGHTS: ReleaseHighlights = {
  version: '0.1.9',
  // getter 防止模块加载时把语言冻结；About 已订阅 locale，重渲染后会重新读取。
  get items() {
    return [
      t('release.0.1.9.1'),
      t('release.0.1.9.2'),
      t('release.0.1.9.3'),
      t('release.0.1.9.4'),
      t('release.0.1.9.5'),
    ]
  },
}
