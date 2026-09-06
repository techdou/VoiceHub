# 声桥 SoundBridge

把小米蓝牙遥控器（RC001 / RC003 / MI RC）变成 Windows 的**无线麦 + 语音遥控器**：

> 按住遥控器语音键说话 → BLE ATVV 音频解码 → VB-CABLE 虚拟声卡 →
> 语音输入工具（SayIt / 微信输入法 / 豆包 / Win+H / 自定义）转文字 → 写入当前聚焦输入框

其余 12 个按键可自定义映射（快捷键 / 媒体 / 音量 / 打开应用网页 / 截图…），
支持双击、长按二级动作，多套键位方案（Profiles）可按前台应用自动切换。
所有数据只存本机，不上传任何东西。

## 功能

- **语音输入**：按住说、松开停；增益可调（±24 dB）；会话自动排空收尾
- **SayIt 联动**：本地 Whisper 听写开箱即用——免提（点一下开始/再点结束，默认）
  与按住说话两种模式可切换，触发键可选（默认右 Alt）
- **按键映射**：12 键 × 单击/双击/长按；录制自定义快捷键；连发支持
- **Profiles**：多套键位方案；Smart Profiles 按前台应用自动切换（默认关闭）
- **一键安装 VB-CABLE**：内置官方下载 + SHA-256 校验，缺失时引导安装
- **统计与历史**：按键次数 / 语音时长 / 最长单次（今日 / 本周 / 近 7 天 / 全部），
  语音会话历史（时间、时长、目标应用、方案）
- **自检**：蓝牙无线电、遥控器配对、VB-CABLE、语音通道、F5 拦截、语音工具指引
- **模拟遥控器页**：无硬件即可验证映射分发与音频链路（真实管线端到端）
- **托盘常驻**：关窗驻留托盘；单实例；开机自启（可选）
- **浅色 / 深色**跟随系统；中文 / English

## 快速开始

**系统要求**：Windows 10 1809+ x64，带蓝牙；一只已配对的小米蓝牙遥控器。

1. 从 [Releases](https://github.com/techdou/soundbridge/releases) 下载安装包
   （`SoundBridge_x64-setup.exe`，或免安装的便携版）
2. 首次启动按引导走：一键安装 VB-CABLE（也可手动装 [官方版](https://vb-audio.com/Cable/)）
   → Windows 蓝牙配对遥控器 → 声桥内选中遥控器与 CABLE Input 端点
3. 选一个语音输入工具，录音设备设为 **CABLE Output**：
   - **SayIt**（本地 Whisper，推荐）：触发键与声桥内所选键保持一致（默认右 Alt）
   - **微信输入法**：语音快捷键设为 Ctrl+Win
   - **豆包 / Win+H / 自定义**：在"连接"页选对应 Provider 即可
4. 按住遥控器语音键说话，文字落进当前输入框

已知边界：遥控器固件单次语音流约 60 秒上限。"连接"页可开启**长录音（实验性）**
尝试续租延长；固件不接受时会正常分段收尾，松开再按即可继续。

## 架构

```
Vue 3 UI（设置窗 / 模拟器）
   ↕ Tauri IPC
宿主编排（src-tauri）：桥管理器 · 语音管线 · 手势分发 · Provider 触发 · 托盘
   ↕
sb-windows（平台层）：WinRT BLE · Raw Input · WH_KEYBOARD_LL 吞键 · SendInput ·
                     WASAPI · 前台进程 · 电源通知 · 蓝牙无线电自愈
   ↕
sb-core（纯逻辑，可测）：ATVV 协议 · IMA ADPCM 编解码 · 帧累积 · PCM 后处理 ·
                         语音会话状态机 · 手势识别 · 映射 · Profiles · 设置迁移 · 统计
```

## 开发

要求：Rust stable（MSVC）、Node 22+、pnpm、WebView2。

```bash
pnpm install
pnpm tauri dev         # 开发运行
pnpm test              # 前端测试（vitest）
cargo test --workspace # Rust 全量单测（sb-core / sb-windows / store）
pnpm tauri build       # 出 NSIS 安装包
```

发布打包用 `scripts/build-release.ps1`（产物进 `artifacts/`，经 GitHub Releases 分发）；
`scripts/ci-preflight.ps1` 做提交前预检。

## 许可

GPL-3.0-only。协议行为参考 techdou/vibe-flow、techdou/remote-mic-app 与
GetSayAll/remote-mic-app-windows（详见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)）。
VB-CABLE 版权归 VB-Audio 所有，按其许可从官方地址下载，不随本软件捆绑。
