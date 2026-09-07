# 第三方声明与致谢

声枢 VoiceHub 的原声桥模块保留 GPL-3.0-only 授权。整合的 SayIt 运行时遵循
AGPL-3.0，组合应用的分发须遵守 AGPL-3.0。

## SayIt 运行时

- crosswk/SayIt 0.1.9，提交 fcb0cc2e9bd62143c861a685fc2fc82226f83c92。
- 版权所有：Liu Qianglong 与 SayIt 贡献者。
- 运行时源码保存在 `vendor/sayit/`，许可证见 `vendor/sayit/LICENSE`，修改说明见
  `vendor/sayit/UPSTREAM.md`。包含语音工作台、识别服务、模型管理、润色、历史与备份。
- transcribe.cpp Rust 绑定与引擎为 MIT 许可；其它依赖保留各自许可证。

## 参考项目

以下开源项目的公开实现为本项目提供了协议事实、架构思路与实测经验参考
（本项目按其公开文档与源码独立重写，未直接复制代码）：

- **techdou/vibe-flow（言灵 Vibe Flow Remote）** — GPL-3.0
  Windows 侧语音管线（ATVV → ADPCM → 电平 → VB-CABLE）、Provider 触发模型
  （微信输入法 Ctrl+Win toggle）、三进程职责划分、自检项设计。
- **techdou/remote-mic-app（无线麦 SayAll）** — GPL-3.0
  产品形态与设置页结构（连接 / 按键 / 统计 / 历史 / 权限 / 关于）、
  按键映射画布、双击 / 长按手势参数（300ms / 550ms）、Smart Profiles、
  ATVV 协议细节（服务 UUID、控制码、能力协商、解码器同步）。
- **GetSayAll/remote-mic-app-windows** — GPL-3.0
  Windows 分层架构（core / windows / tauri host）、WinRT BLE 用法、
  F5 吞键闸防粘键设计、蓝牙无线电自愈策略、wasapi crate 用法。

## 依赖

- [Tauri 2](https://tauri.app)（MIT/Apache-2.0）与各插件
- [wasapi crate](https://crates.io/crates/wasapi)（MPL-2.0 / 部分 LGPL）
- [windows crate](https://crates.io/crates/windows)（MIT/Apache-2.0）
- Vue 3 / Vite / Vitest（MIT）
- VB-CABLE 为 VB-Audio 的第三方免费软件，不随本项目分发。

## 硬件

小米蓝牙遥控器 2（RC001）/ 2 Pro（RC003）为小米公司产品，本项目与其无关联。
