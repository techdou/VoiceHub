# 声桥 SoundBridge

把小米蓝牙遥控器（RC001 / RC003 / MI RC）变成 Windows 的**无线麦 + 语音遥控器**：

> 按住遥控器语音键说话 → BLE ATVV 音频解码 → VB-CABLE 虚拟声卡 →
> 第三方语音输入法（微信输入法 / 豆包 / Win+H / 自定义）转文字 → 写入当前聚焦输入框

其余 12 个按键可自定义映射（快捷键 / 媒体 / 音量 / 打开应用网页 / 截图…），
支持双击、长按二级动作，多套键位方案（Profiles）可按前台应用自动切换。
所有数据只存本机。

## 功能

- **语音输入**：按住说、松开停；增益可调（±24 dB）；会话自动排空收尾
- **按键映射**：12 键 × 单击/双击/长按；录制自定义快捷键；连发支持
- **Profiles**：多套键位方案；Smart Profiles 按前台应用自动切换（默认关闭）
- **统计与历史**：按键次数 / 语音时长 / 最长单次（今日 / 本周 / 近 7 天 / 全部），
  语音会话历史（时间、时长、目标应用、方案）
- **自检**：蓝牙无线电、遥控器配对、VB-CABLE、语音通道、F5 拦截、语音工具指引
- **模拟遥控器页**：无硬件即可验证映射分发与音频链路（真实管线端到端）
- **托盘常驻**：关窗驻留托盘；单实例；开机自启（可选）
- **浅色 / 深色**跟随系统；中文 / English

## 架构

```
Vue 3 UI（设置窗 / 模拟器）
   ↕ Tauri IPC
宿主编排（src-tauri）：桥管理器 · 语音管线 · 手势分发 · Provider 触发 · 托盘
   ↕
sb-windows（平台层）：WinRT BLE · Raw Input · WH_KEYBOARD_LL 吞键 · SendInput ·
                     WASAPI(wasapi crate) · 前台进程 · 电源通知 · 蓝牙无线电自愈
   ↕
sb-core（纯逻辑，可测）：ATVV 协议 · IMA ADPCM 编解码 · 帧累积 · PCM 后处理 ·
                         语音会话状态机 · 手势识别 · 映射 · Profiles · 设置迁移 · 统计
```

## 开发

要求：Windows 10 1809+ x64、Rust stable（MSVC）、Node 22+、pnpm、WebView2。

```bash
pnpm install
pnpm test              # 前端测试（vitest）
cargo test --workspace # Rust 全量单测（sb-core / sb-windows / store）
pnpm tauri dev         # 开发运行
pnpm tauri build       # 出 NSIS 安装包
```

使用流程：安装 [VB-CABLE](https://vb-audio.com/Cable/) → Windows 蓝牙配对遥控器 →
声桥内选择遥控器与 CABLE Input 端点 → 语音工具录音设备设为 CABLE Output →
微信输入法语音快捷键设为 Ctrl+Win → 按住遥控器语音键说话。

## 许可

GPL-3.0-only。协议行为参考 techdou/vibe-flow、techdou/remote-mic-app 与
GetSayAll/remote-mic-app-windows（详见 THIRD_PARTY_NOTICES.md）。
