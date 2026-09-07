import { ArrowUpRight, Github, Globe } from 'lucide-react'
import { open } from '@tauri-apps/plugin-shell'
import appIcon from '../assets/voicehub.svg'
import { useT } from '@/i18n/useT'

interface CreditGroup {
  titleKey: string
  credits: Credit[]
}

interface Credit {
  name: string
  roleKey: string
  license: string
  url: string
  web?: boolean
}

// 链接与许可均经 gh api 核实（2026-09-07）。
const CREDIT_GROUPS: CreditGroup[] = [
  {
    titleKey: 'about.groupEngine',
    credits: [
      { name: 'SayIt', roleKey: 'about.credit.sayit', license: 'AGPL-3.0', url: 'https://github.com/crosswk/SayIt' },
      { name: 'whisper.cpp / GGML', roleKey: 'about.credit.whisper', license: 'MIT', url: 'https://github.com/ggerganov/whisper.cpp' },
      { name: 'Silero VAD', roleKey: 'about.credit.silero', license: 'MIT', url: 'https://github.com/snakers4/silero-vad' },
    ],
  },
  {
    titleKey: 'about.groupProtocol',
    credits: [
      { name: 'vibe-flow（言灵）', roleKey: 'about.credit.vibeFlow', license: 'GPL-3.0', url: 'https://github.com/techdou/vibe-flow' },
      { name: 'remote-mic-app（SayAll）', roleKey: 'about.credit.remoteMic', license: 'GPL-3.0', url: 'https://github.com/techdou/remote-mic-app' },
      { name: 'remote-mic-app-windows', roleKey: 'about.credit.remoteMicWin', license: 'GPL-3.0', url: 'https://github.com/GetSayAll/remote-mic-app-windows' },
    ],
  },
  {
    titleKey: 'about.groupSystem',
    credits: [
      { name: 'VB-CABLE', roleKey: 'about.credit.vbcable', license: '', url: 'https://vb-audio.com/Cable/', web: true },
    ],
  },
]

export default function About() {
  const t = useT()
  return <div className="mx-auto max-w-4xl space-y-6">
    <div className="flex items-center gap-4 border-b pb-6">
      <img src={appIcon} alt="" className="h-16 w-16" />
      <div><h1 className="text-2xl font-bold">声枢 VoiceHub</h1><p className="mt-1 text-sm text-muted-foreground">v{__APP_VERSION__}</p></div>
    </div>
    <section className="space-y-3">
      <h2 className="text-base font-semibold">{t('about.creditsTitle')}</h2>
      <p className="text-sm text-muted-foreground">{t('about.description')}</p>
      <div className="space-y-4">
        {CREDIT_GROUPS.map(group => (
          <div key={group.titleKey}>
            <p className="mb-1.5 text-xs font-medium text-muted-foreground">{t(group.titleKey as never)}</p>
            <div className="divide-y rounded-lg border">
              {group.credits.map(credit => (
                <button
                  key={credit.url}
                  className="flex w-full items-center gap-3 px-4 py-3 text-left hover:bg-muted/40"
                  onClick={() => void open(credit.url)}
                >
                  {credit.web ? <Globe size={16} className="shrink-0 text-muted-foreground" /> : <Github size={16} className="shrink-0 text-muted-foreground" />}
                  <span className="min-w-0 flex-1">
                    <span className="block text-sm font-medium">{credit.name}</span>
                    <span className="block text-xs text-muted-foreground">{t(credit.roleKey as never)}</span>
                  </span>
                  {credit.license && (
                    <span className="shrink-0 rounded border border-border px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
                      {credit.license}
                    </span>
                  )}
                  <ArrowUpRight size={14} className="shrink-0 text-muted-foreground" />
                </button>
              ))}
            </div>
          </div>
        ))}
      </div>
    </section>
  </div>
}
