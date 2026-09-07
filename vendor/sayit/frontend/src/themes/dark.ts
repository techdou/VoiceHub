import type { ThemeDefinition } from './types'
import { t } from '@/i18n'

/**
 * 暗色主题 — VS Code 风格
 * 低饱和度中性灰，蓝色强调，文字对比度充足
 */
const dark: ThemeDefinition = {
  id: 'dark',
  get name() { return t('theme.dark') },
  isDark: true,
  previewColors: {
    bg: '#1e1e1e',
    sidebar: '#181818',
    primary: '#569cd6',
    accent: '#2d2d2d',
  },
  vars: {
    // 基础色 — 中性灰
    '--background': '0 0% 12%',
    '--foreground': '0 0% 90%',
    '--card': '0 0% 15%',
    '--card-foreground': '0 0% 90%',
    '--primary': '210 60% 58%',            // VS Code 蓝
    '--primary-foreground': '0 0% 100%',
    '--secondary': '0 0% 18%',
    '--secondary-foreground': '0 0% 88%',
    '--muted': '0 0% 18%',
    '--muted-foreground': '0 0% 65%',
    '--accent': '0 0% 20%',
    '--accent-foreground': '0 0% 90%',
    '--destructive': '0 60% 55%',
    '--destructive-foreground': '0 0% 95%',
    '--destructive-strong': '0 70% 68%',    // #1f1f1f 底 5.6:1
    '--border': '0 0% 22%',
    '--input': '0 0% 22%',
    '--ring': '210 60% 58%',
    '--radius': '0.5rem',

    // 区域色
    '--sidebar-bg': '0 0% 10%',
    '--sidebar-border': '0 0% 18%',
    '--sidebar-item-active-bg': '0 0% 18%',
    '--sidebar-item-hover-bg': '0 0% 16%',
    '--sidebar-text': '0 0% 60%',
    '--sidebar-text-active': '0 0% 92%',
    '--titlebar-bg': '0 0% 10%',
    '--titlebar-text': '0 0% 65%',
    '--titlebar-close-hover-bg': '0 60% 50%',
    '--titlebar-close-hover-text': '0 0% 100%',

    // 表单控件
    '--input-bg': '0 0% 13%',
    '--input-border': '0 0% 22%',
    '--input-focus-border': '210 60% 58%',
    '--input-focus-ring': '210 60% 58%',
    '--input-placeholder': '0 0% 45%',

    // 状态色（暗底上 success/warning/info 本身已达标，-strong 仅略提亮）
    '--success': '120 40% 55%',
    '--success-foreground': '0 0% 100%',
    '--success-strong': '120 45% 62%',      // 6.6:1 → 8.0:1
    '--warning': '40 80% 60%',
    '--warning-foreground': '0 0% 10%',
    '--warning-strong': '40 90% 65%',       // 8.7:1 → 10.0:1
    '--info': '210 60% 58%',
    '--info-foreground': '0 0% 100%',
    '--info-strong': '210 70% 68%',         // 5.1:1 → 7.0:1
  },
}

export default dark
