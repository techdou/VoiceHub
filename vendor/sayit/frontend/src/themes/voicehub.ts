import light from './light'
import type { ThemeDefinition } from './types'

const voicehub: ThemeDefinition = {
  ...light,
  id: 'voicehub',
  name: '声枢',
  previewColors: { bg: '#fafbfc', sidebar: '#f0f3f5', primary: '#16745b', accent: '#e4f2ed' },
  fonts: { body: '"Segoe UI Variable Text", "Microsoft YaHei UI", sans-serif' },
  vars: {
    ...light.vars,
    '--background': '210 20% 99%', '--foreground': '202 20% 14%',
    '--primary': '162 68% 27%', '--primary-foreground': '0 0% 100%',
    '--muted': '210 17% 95%', '--muted-foreground': '210 9% 43%',
    '--accent': '158 30% 93%', '--accent-foreground': '162 68% 23%',
    '--border': '210 16% 88%', '--ring': '162 68% 27%',
    '--sidebar-bg': '210 18% 96%', '--sidebar-text': '210 10% 39%',
    '--sidebar-border': '210 16% 88%', '--sidebar-text-active': '162 68% 23%',
    '--sidebar-item-active-bg': '158 32% 90%', '--sidebar-item-hover-bg': '210 16% 92%',
    '--titlebar-bg': '0 0% 100%', '--titlebar-text': '202 20% 14%',
    '--input-focus-border': '162 68% 27%', '--input-focus-ring': '162 68% 27%',
  },
}
export default voicehub
