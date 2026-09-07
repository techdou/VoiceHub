import { ArrowUpRight, Github } from 'lucide-react'
import { open } from '@tauri-apps/plugin-shell'
import appIcon from '../assets/voicehub.svg'

interface Credit {
  name: string
  role: string
  license: string
  url: string
}

// 链接与许可均经 gh api 核实（2026-09-07）。
const CREDIT_GROUPS: { title: string; credits: Credit[] }[] = [
  {
    title: '引擎内核',
    credits: [
      { name: 'SayIt', role: '语音工作区（内嵌引擎的直接上游）', license: 'AGPL-3.0', url: 'https://github.com/crosswk/SayIt' },
      { name: 'whisper.cpp / GGML', role: '本地语音识别底座', license: 'MIT', url: 'https://github.com/ggerganov/whisper.cpp' },
      { name: 'Silero VAD', role: '语音活动检测（自动断句）', license: 'MIT', url: 'https://github.com/snakers4/silero-vad' },
    ],
  },
  {
    title: '协议参考',
    credits: [
      { name: 'vibe-flow（言灵）', role: 'ATVV 蓝牙语音协议参考', license: 'GPL-3.0', url: 'https://github.com/techdou/vibe-flow' },
      { name: 'remote-mic-app（SayAll）', role: '蓝牙麦克风协议参考', license: 'GPL-3.0', url: 'https://github.com/techdou/remote-mic-app' },
      { name: 'remote-mic-app-windows', role: 'Windows 实现参考', license: 'GPL-3.0', url: 'https://github.com/GetSayAll/remote-mic-app-windows' },
    ],
  },
  {
    title: '系统组件',
    credits: [
      { name: 'VB-CABLE', role: '虚拟声卡（免费/donationware）', license: '', url: 'https://vb-audio.com/Cable/' },
    ],
  },
]

export default function About() {
  return <div className="mx-auto max-w-4xl space-y-6">
    <div className="flex items-center gap-4 border-b pb-6">
      <img src={appIcon} alt="" className="h-16 w-16" />
      <div><h1 className="text-2xl font-bold">声枢 VoiceHub</h1><p className="mt-1 text-sm text-muted-foreground">v{__APP_VERSION__}</p></div>
    </div>
    <section className="space-y-3">
      <h2 className="text-base font-semibold">开源致谢</h2>
      <p className="text-sm text-muted-foreground">语音工作区基于 SayIt 0.1.9，原作者 Liu Qianglong。集成版本按 AGPL-3.0 提供；原声桥硬件模块保留 GPL-3.0 声明。完整依赖清单见仓库 THIRD_PARTY_NOTICES。</p>
      <div className="space-y-4">
        {CREDIT_GROUPS.map(group => (
          <div key={group.title}>
            <p className="mb-1.5 text-xs font-medium text-muted-foreground">{group.title}</p>
            <div className="divide-y rounded-lg border">
              {group.credits.map(credit => (
                <button
                  key={credit.url}
                  className="flex w-full items-center gap-3 px-4 py-3 text-left hover:bg-muted/40"
                  onClick={() => void open(credit.url)}
                >
                  <Github size={16} className="shrink-0 text-muted-foreground" />
                  <span className="min-w-0 flex-1">
                    <span className="block text-sm font-medium">{credit.name}</span>
                    <span className="block text-xs text-muted-foreground">{credit.role}</span>
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
