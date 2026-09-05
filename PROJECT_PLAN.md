# 声桥 SoundBridge — 小米蓝牙遥控器 → Windows 语音输入桥

把小米蓝牙遥控器（RC001 / RC003 / MI RC）变成 Windows 的"无线麦 + 语音遥控器"：
按住语音键说话 → 解码 BLE ATVV 音频 → 播入 VB-CABLE 虚拟声卡 → 第三方语音输入法
（微信输入法 / 豆包 / Win+H / 自定义）转文字 → 写入当前聚焦输入框。
其余 12 个按键可自定义映射（快捷键 / 媒体 / 音量 / 打开应用或网页 / 截图等），
支持双击、长按二级动作，多套键位方案（Profiles）可按前台应用自动切换。

## 参考项目（refs/ 下，不参与构建）

| 仓库 | 用途 |
| --- | --- |
| techdou/remote-mic-app（macOS SayAll） | **功能与界面主参考**：设置页结构（连接/按键/统计/历史/权限/关于）、按键映射画布、动作分类、双击/长按手势、App Profiles、统计、回眸历史 |
| GetSayAll/remote-mic-app-windows | 架构参考：Rust + Tauri 2 + Vue 3 分层（core / windows / tauri host）、WinRT BLE、WASAPI、Raw Input + SendInput |
| techdou/vibe-flow | Windows 落地参考：三进程职责划分、语音管线（ADPCM→电平→重采样→CABLE）、Provider 触发（微信输入法 Ctrl+Win toggle）、自检项、Profiles 工作流 |

协议事实（ATVV over BLE GATT，来自上述实现交叉验证）：
- 服务 `AB5E0001-5A21-4F05-BC7D-AF01F617B664`；特征：transmit(0002, 写)、audio(0003, notify)、control(0004, notify)
- 控制码：0x0A GetCapabilities / DecoderSync、0x0B Capabilities、0x08 设备请求开麦、
  0x04 StreamStart{interaction,codec,session}、0x00 StreamStop；主机命令 0x0C 开麦 / 0x0D 关麦 / 0x0E 延长
- 音频：IMA ADPCM，RC001/RC003 高半字节优先（ARN9 固件低半字节优先，按 2A24 型号识别翻转），
  codec 0x02 = 16 kHz；帧长默认 120 字节；流中 0x0A 携带解码器同步（predictor/stepIndex）
- HID：遥控器按键 = Consumer/Keyboard usage 数组报文（report ID 1，小端 u16 数组）；
  语音键 = 键盘 F5 (page7 usage 0x3E)，Xiaomi VID 0x2717 / PID 0x32B8
- 按键 usage 表：power 0x66, up 0x52, left 0x50, ok 0x28, right 0x4F, down 0x51,
  back 0xF1, vol+ 0x80, home 0x4A, vol- 0x81, menu 0x65, tv 0x35

## 技术选型

- **Rust + Tauri 2 + Vue 3 + TypeScript + Vite**（与参考 Windows 版一致；本机 rustc 1.96-msvc / Node 22 / pnpm / WebView2 齐备）
- 分层：`crates/sb-core`（纯逻辑，可测）→ `crates/sb-windows`（平台 API）→ `src-tauri`（编排+IPC）→ `src`（Vue UI）
- 许可：GPL-3.0-only（与参考一致，附 THIRD_PARTY_NOTICES 致谢）

## 阶段与交付（P0 → P6）

- **P0 脚手架**：cargo workspace + pnpm workspace + Vite + Tauri 骨架可启动
- **P1 核心库 sb-core**（纯逻辑 + 全量单测）：ATVV 编解码、IMA ADPCM、帧累积器、PCM 后处理、
  语音会话状态机、按键手势识别（单击/双击 300ms/长按 550ms）、动作模型与映射、
  Profiles（多方案 + 前台应用匹配 + 回退）、设置模型与迁移、统计聚合（今日/本周/全部/近7天）
- **P2 平台库 sb-windows**：WinRT BLE（配对扫描/连接/GATT/通知）、Raw Input（按设备路径过滤）、
  低级键盘钩子（吞掉遥控器原生按键）、SendInput（快捷键/媒体/音量）、WASAPI 输出到所选端点、
  前台进程检测、电源（睡眠/唤醒）通知
- **P3 宿主编排 src-tauri**：桥管理器（状态机+指数退避重连+代际防串）、语音管线（音频→端点+Provider 触发：
  微信输入法 toggle / 豆包 hold / Win+H / 自定义）、按键分发（动作执行+回执）、托盘、单实例、开机自启、日志
- **P4 UI**：Mac 风格设置窗（侧边栏：连接/按键/统计/历史/自检/关于）、遥控器可视化映射画布、
  动作选择器（基础键/系统媒体/自定义/应用）、快捷键录制、Profiles 管理、首次引导向导、
  **模拟遥控器页（无硬件端到端验证）**、浅色/深色主题、中英文
- **P5 打包与脚本**：构建脚本、NSIS 安装包 + 便携 ZIP、ci-preflight 预检
- **P6 验证**：cargo test / vitest 全绿、`cargo build` 出 exe、模拟遥控器端到端（按钮事件+合成 ATVV 音频→波形输出）跑通、
  输出真机验收清单（需豆哥拿 RC003 实测）

## 真机验收（硬件到位后由豆哥执行，清单在 docs/HARDWARE_ACCEPTANCE.md）

配对连接、按住说话→微信输入法出字、12 键映射、双击/长按、Profile 自动切换、断连重连、睡眠唤醒恢复、
60 秒会话边界、VB-CABLE 缺失提示、电池/型号识别。
