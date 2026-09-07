import { Github } from 'lucide-react'
import { open } from '@tauri-apps/plugin-shell'
import appIcon from '../assets/voicehub.svg'

export default function About() {
  return <div className="mx-auto max-w-4xl space-y-6">
    <div className="flex items-center gap-4 border-b pb-6">
      <img src={appIcon} alt="" className="h-16 w-16" />
      <div><h1 className="text-2xl font-bold">声枢 VoiceHub</h1><p className="mt-1 text-sm text-muted-foreground">v{__APP_VERSION__}</p></div>
    </div>
    <section className="space-y-3">
      <h2 className="text-base font-semibold">开源致谢</h2>
      <p className="text-sm text-muted-foreground">语音工作区基于 SayIt 0.1.9，原作者 Liu Qianglong。集成版本按 AGPL-3.0 提供；原声桥硬件模块保留 GPL-3.0 声明。</p>
      <button className="flex items-center gap-2 text-sm underline" onClick={() => void open('https://github.com/crosswk/SayIt')}><Github size={16} />SayIt 源码与许可</button>
    </section>
  </div>
}
