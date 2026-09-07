<p align="center">
  <img src="src/assets/voicehub.svg" width="80" height="80" alt="VoiceHub Logo" />
</p>

# 声枢 VoiceHub

声枢是面向 Windows 的语音输入工作区，整合原声桥的蓝牙遥控器与按键映射，以及 SayIt 0.1.9 的录音、识别、AI 整理和文本填入功能。默认直接传输遥控器音频，无需运行独立 SayIt，也无需安装虚拟声卡。

```text
遥控器语音键 → 蓝牙音频 → 声枢录音会话 → 本地或第三方 ASR → 当前输入框
遥控器按键   → 设备来源校验 → 按键统计与自定义映射
```

## 功能

- 本地 GGUF 语音模型：内置模型下载、已有模型目录、自定义模型文件路径、加载验证、语言和加速器选择。
- 第三方 ASR：保留 SayIt 内置供应商，新增 OpenAI 兼容转写接口，自填地址、模型名和可选密钥；支持本机 HTTP 服务。
- SayIt 语音设置：麦克风与快捷键、服务器连接、AI 服务与指令、热词、替换规则、历史重识别、收藏、导出和备份。
- 独立声枢 Logo、统一工作区和主题。应用生命周期由声枢管理，不使用独立 SayIt 的自动更新器。
- 遥控器语音输入：音频解码、增益调节、电平显示与会话历史。
- 按键映射：快捷键、媒体控制、音量、打开应用等动作，支持单击、双击和长按。
- 多套键位方案，可按前台应用自动选择方案。
- 使用统计：按键次数、语音会话数、累计时长、最长单次时长，支持今日、本周、近 7 天和全部范围。
- 连接与音频自检、虚拟声卡安装引导、托盘常驻和可选开机自启。
- 模拟遥控器，用于检查映射与音频链路。

## 环境要求

| 项目 | 要求 |
| --- | --- |
| 操作系统 | Windows 10 1809 或更高版本，x64 |
| 蓝牙 | 支持低功耗蓝牙（Bluetooth Low Energy，BLE）的适配器 |
| 遥控器 | 已在 Windows 中配对的小米蓝牙语音遥控器；不同型号和固件需分别验证 |
| 窗口运行环境 | Microsoft Edge WebView2 Runtime |
| 音频桥接 | 内嵌模式无需虚拟声卡；外部输入法兼容模式需要 VB-CABLE |
| 文字识别 | 内嵌本地模型、第三方 API、自部署服务器，或外部输入法兼容模式 |

自定义本地文件必须是 transcribe.cpp 支持的语音识别 GGUF，不能把任意聊天模型或 ONNX 权重直接当作 ASR 加载。其它模型可通过本机 OpenAI 兼容服务接入。

## 安装与使用

1. 从 [GitHub Releases](https://github.com/techdou/soundbridge/releases) 获取已发布的安装包或便携版。主分支的新修复可能尚未包含在旧安装包中，也可以按下文从源码构建。
2. 在 Windows 蓝牙设置中配对遥控器，在“设备连接”页选择设备，语音工具选择“声枢内嵌引擎”并保存。
3. 在“语音引擎”选择本地模式，下载模型或选择已有 GGUF 文件并点击“加载并启用”。
4. 使用第三方或本机服务时，选择云 API 模式，添加“自定义 ASR / OpenAI 兼容”，填写服务地址与模型名，按服务要求填写密钥。
5. 运行识别测试；也可在“模拟遥控器”选择自己的音频文件测试整条录音链路。
6. 聚焦目标输入框，按住遥控器语音键说话，松开后检查文字输入结果。

### 自定义 ASR 配置

在“云 API 模式”新建服务，选择“自定义 ASR / OpenAI 兼容”：

| 字段 | 示例 / 要求 |
| --- | --- |
| 接口地址 | `http://127.0.0.1:8000/v1`，也可填写完整的 `/v1/audio/transcriptions` URL |
| 模型名称 | 服务实际支持的模型标识，例如 `whisper-1` |
| API Key | 服务要求鉴权时填写；无鉴权的本机服务可留空 |

客户端将音频封装为 WAV，通过 multipart `POST` 请求提交 `file`、`model` 和 `response_format=json`。配置密钥时使用 Bearer 鉴权。服务需要返回包含 `text` 字段的 JSON，例如：

```json
{"text":"识别出的文字"}
```

基础地址会自动补齐 `/audio/transcriptions`，完整接口地址不会重复追加。协议不兼容的服务需要适配，不能仅通过填写模型名称直接使用。内嵌模式不需要配置 SayIt 联动快捷键；外部输入法兼容模式仍使用虚拟声卡和其原有快捷键。

语音配置保存在 `%LOCALAPPDATA%\app.soundbridge.windows\sayit`，原硬件配置路径不变。独立 SayIt 的数据和密钥不会自动复制，可使用设置中的备份导入。自定义模型直接读取原文件，不复制或删除模型权重。

在“按键映射”页配置遥控器动作。关闭“启用按键映射”可以停止声枢执行映射，但不会关闭遥控器使用统计。

关闭窗口会隐藏到托盘。需要完全停止声枢或更换版本时，请从托盘菜单选择退出，再启动新版本。

便携版须保留可执行文件旁的 DLL 与 `resources/`，不要仅复制 EXE。没有遥控器时，可在设置中选择电脑麦克风和录音快捷键。

## 统计范围与设备隔离

- 按键统计基于已识别的遥控器按下事件；一次长按或双击的动作触发数不等于物理按下次数。
- 输入来源必须是当前选中遥控器对应的蓝牙 HID（人机接口设备），通过设备路径和蓝牙地址校验后才进入统计与映射。
- 普通鼠标、键盘和未选中的设备不计入遥控器按键统计。仅厂商品牌相同不构成匹配条件。
- 语音统计记录会话数与时长，不代表识别字数、识别准确率或识别成功次数。
- 模拟器会实际执行映射和语音链路；模拟语音会话也会进入语音统计。
- 旧版误计的数据不会被自动回溯修正，升级保留已有统计和历史。

## 数据与隐私

为兼容已有配置，应用标识仍为 `app.soundbridge.windows`。硬件配置、统计、会话历史和宿主日志保存在本机：

```text
%APPDATA%\app.soundbridge.windows\
  settings.json
  statistics.json
  history.jsonl
  logs\
```

内嵌语音设置、识别历史、录音及默认模型保存在 `%LOCALAPPDATA%\app.soundbridge.windows\sayit\`。API 密钥保存在本机设置中，配置导出或备份可能包含凭据。请妥善保管，不要提交到 Git 或公开分享。

声枢不主动上传这些使用记录。配置和日志可能包含设备标识、前台应用名称等本地信息；提交问题报告前应先脱敏，不要直接上传整个数据目录。云端识别会将录音发往所配置的服务，AI 整理会将文本发往所配置的 AI 服务，启用 WebDAV 备份时会上传所选择的备份数据。

安装虚拟声卡需要下载第三方安装包，语音输入软件也可能联网或向其服务商发送音频与文本。请根据所选工具的设置和隐私政策判断数据去向。

## 从源码构建

准备 Rust stable（MSVC 工具链）、Visual Studio 2022 C++ Build Tools、Windows SDK、Node.js 22+ 和 pnpm。安装 C++ 桌面开发与 CMake 工具组件。首次构建会编译本地推理依赖并下载所需资源。

```powershell
git clone https://github.com/techdou/soundbridge.git
cd soundbridge
pnpm install --frozen-lockfile
pnpm build
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-native.ps1 -Action build -Standalone
.\target\debug\soundbridge-app.exe
```

`-Standalone` 将前端资源嵌入应用，可脱离 Vite 启动。构建脚本负责初始化 MSVC、CMake 与本地推理库的构建环境。

### 开发与验证

前端热更新需要两个终端。第一个运行 `pnpm dev`；第二个执行以下命令，生成使用开发服务器的原生应用：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-native.ps1 -Action build
.\target\debug\soundbridge-app.exe
```

默认开发地址为 `http://localhost:5173`。网页依赖 Tauri 原生接口，应通过桌面应用验收。

运行测试与前端构建：

```powershell
pnpm test
pnpm build
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-native.ps1 -Action test
```

也可执行 `scripts/ci-preflight.ps1`，依次检查依赖安装、前端构建、前端测试、原生测试和独立应用构建。重新编译前先退出正在使用目标目录 DLL 的应用实例。

### 发布构建

```powershell
# NSIS 安装包及便携 ZIP，附 SHA-256 校验文件
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-release.ps1

# 只生成便携版
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-release.ps1 -SkipInstaller
```

发布构建通过 Tauri CLI 打包前端资源。可执行文件位于 `target/release/`，NSIS 安装包位于 `target/release/bundle/nsis/`。整理后的发行产物位于 `artifacts/`，不纳入源码版本控制。GPU 加速为可选构建能力，需要匹配的工具链、后端与驱动；默认构建支持 CPU 推理。

## 目录结构

```text
crates/
  sb-core/       协议、音频解码、手势、映射与统计逻辑
  sb-windows/    蓝牙、原始输入、按键注入与 Windows 音频接口
src/             硬件设置的 Vue 页面、共享资源与测试
vendor/sayit/
  frontend/      内嵌语音工作区与前端测试
  native/        识别、模型、文本输入、存储与原生测试
  LICENSE        上游许可证
  UPSTREAM.md    上游版本与修改归属声明
src-tauri/       桌面应用编排、命令、托盘及持久化
public/          应用静态资源
scripts/         构建与验证脚本
README.md        公开使用说明
LICENSE          项目许可证
THIRD_PARTY_NOTICES.md  第三方声明
```

开发计划与内部文档放在本地 `docs/`，构建产物放在 `artifacts/`，均不纳入版本控制。依赖目录、编辑器与代理配置、录音、模型、数据库、缓存、密钥和证书由 `.gitignore` 排除。README 和许可 / 上游归属说明是公开保留的例外；运行必需的图标和内置测试音频随源码保留。忽略规则不会移除已经提交到 Git 历史中的内容。

## 已知限制与排查

- 不同遥控器型号、固件和 HID 报文格式存在差异，不能保证所有实体按键均可识别。设备过滤的回归测试不能替代实体遥控器逐键和语音验证。
- 遥控器固件可能限制单次语音流时长。“长录音（实验性）”尝试续租，但不能保证突破固件限制。
- 直接 PCM 传输与自定义兼容 ASR 单段上限为五分钟。自动化测试不能替代实体遥控器长录音、不同 GPU 或所有云供应商账号的实机验证。
- 文字未填入时，先确认目标输入框已聚焦。管理员权限窗口、安全输入框及特殊编辑器可能限制模拟输入，应结合诊断日志排查。
- 内嵌模式无音频时，先检查遥控器连接状态、电平与所选识别模型。只有外部输入法兼容模式才需要核对 `CABLE Input` / `CABLE Output` 和虚拟声卡占用情况。
- 遇到鼠标操作导致统计增加或额外确认，请先退出旧版声桥并确认运行的是修正版；不要同时运行不同版本。
- `crates/sb-windows/examples/check_input_sources.rs` 是只读输入来源诊断工具，可列出设备并显示筛选结果。输出包含设备路径，分享前需要脱敏。

## 许可证与第三方组件

整合后的声枢按 [AGPL-3.0](vendor/sayit/LICENSE) 分发；原声桥硬件模块保留 [GPL-3.0-only](LICENSE) 声明。第三方组件与参考项目见 [第三方声明](THIRD_PARTY_NOTICES.md)。VB-CABLE 为 VB-Audio 的第三方软件，不随本仓库分发。小米遥控器为其所属公司的产品，本项目与小米无隶属关系。
