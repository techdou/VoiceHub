//! 按键动作模型（Windows 语义：VK 键码 / 媒体键 / 系统命令 / 应用与网页）。
//!
//! 动作分四类（对齐参考实现的分组）：基础按键、系统与媒体、自定义、应用。
//! 动作到 SendInput / ShellExecute 的翻译在 sb-windows 层实现。

use serde::{Deserialize, Serialize};

/// Windows 虚拟键码（动作需要的子集）。
pub mod vk {
    pub const RETURN: u16 = 0x0D;
    pub const ESCAPE: u16 = 0x1B;
    pub const SPACE: u16 = 0x20;
    pub const END: u16 = 0x23;
    pub const HOME: u16 = 0x24;
    pub const LEFT: u16 = 0x25;
    pub const UP: u16 = 0x26;
    pub const RIGHT: u16 = 0x27;
    pub const DOWN: u16 = 0x28;
    pub const INSERT: u16 = 0x2D;
    pub const DELETE: u16 = 0x2E;
    pub const A: u16 = 0x41;
    pub const C: u16 = 0x43;
    pub const F: u16 = 0x46;
    pub const H: u16 = 0x48;
    pub const L: u16 = 0x4C;
    pub const N: u16 = 0x4E;
    pub const P: u16 = 0x50;
    pub const S: u16 = 0x53;
    pub const V: u16 = 0x56;
    pub const W: u16 = 0x57;
    pub const X: u16 = 0x58;
    pub const Y: u16 = 0x59;
    pub const Z: u16 = 0x5A;
    pub const F2: u16 = 0x71;
    pub const F3: u16 = 0x72;
    pub const F5: u16 = 0x74;
    pub const LWIN: u16 = 0x5B;
    pub const RWIN: u16 = 0x5C;
    pub const LCONTROL: u16 = 0xA2;
    pub const LSHIFT: u16 = 0xA0;
    pub const LMENU: u16 = 0xA4; // Alt
    pub const TAB: u16 = 0x09;
    pub const BROWSER_BACK: u16 = 0xA6;
    pub const BROWSER_FORWARD: u16 = 0xA7;
    pub const BACK: u16 = 0x08; // Backspace
    pub const PRIOR: u16 = 0x21; // PgUp
    pub const NEXT: u16 = 0x22; // PgDn
}

/// 修饰键位标志（与 Win32 MOD_* 对齐）。
pub const MOD_ALT: u8 = 0x01;
pub const MOD_CONTROL: u8 = 0x02;
pub const MOD_SHIFT: u8 = 0x04;
pub const MOD_WIN: u8 = 0x08;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionCategory {
    BasicKeys,
    SystemAndMedia,
    Custom,
    Applications,
}

/// 自定义快捷键（录制自真实键盘）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomShortcut {
    pub vk: u16,
    pub modifiers: u8,
    /// 展示名，如 "Ctrl+Shift+K"。
    pub label: String,
}

impl CustomShortcut {
    pub fn new(vk: u16, modifiers: u8, label: impl Into<String>) -> Self {
        Self { vk, modifiers, label: label.into() }
    }
}

/// 可绑定到遥控器按键的动作。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ButtonAction {
    Disabled,
    /// 单键或带修饰键的组合。
    Shortcut { vk: u16, modifiers: u8, label: String },
    /// 媒体 / 系统键（SendInput 虚拟键扫描码路径）。
    MediaKey { code: MediaKeyCode },
    VolumeUp,
    VolumeDown,
    VolumeMute,
    /// 打开（或切换到）已装应用：可执行路径或 URI scheme。
    OpenApp { target: String, label: String },
    /// 打开 HTTPS 网页（默认浏览器）。
    OpenUrl { url: String },
    /// 截图（全屏到剪贴板 / Win+Shift+S 区域截图）。
    Screenshot { region: bool },
    /// 显示桌面（Win+D）。
    ShowDesktop,
    /// 任务视图（Win+Tab）。
    TaskView,
    /// 切窗口（Alt+Tab 单步）。
    AppSwitcher,
    /// 对话框模拟左键（供 AI 客户端“继续”按钮）。
    ClickConfirm,
    /// 打开声桥设置窗。
    OpenSettings,
    /// 自定义快捷键（按键粒度引用，值存 mapping 的 shortcuts 表）。
    Custom { shortcut: CustomShortcut },
    /// 免提触发（事件直连引擎 toggle-hands-free，不注入按键、不依赖引擎快捷键配置）。
    TriggerHandsFree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKeyCode {
    PlayPause,
    Stop,
    Next,
    Previous,
    Mute,
}

/// 预设动作（动作选择器中的条目，映射到 ButtonAction）。
pub struct PresetAction {
    pub id: &'static str,
    pub category: ActionCategory,
    pub label_zh: &'static str,
    pub label_en: &'static str,
    pub action: ButtonAction,
}

/// 内置预设动作表（UI 动作选择器与默认键位共用）。
pub fn preset_actions() -> Vec<PresetAction> {
    use ActionCategory::*;
    let s = |vk: u16, m: u8, label: &str| ButtonAction::Shortcut {
        vk,
        modifiers: m,
        label: label.into(),
    };
    vec![
        PresetAction { id: "escape", category: BasicKeys, label_zh: "Esc", label_en: "Esc", action: s(vk::ESCAPE, 0, "Esc") },
        PresetAction { id: "enter", category: BasicKeys, label_zh: "回车", label_en: "Enter", action: s(vk::RETURN, 0, "Enter") },
        PresetAction { id: "ctrl_enter", category: BasicKeys, label_zh: "Ctrl+回车", label_en: "Ctrl+Enter", action: s(vk::RETURN, MOD_CONTROL, "Ctrl+Enter") },
        PresetAction { id: "shift_enter", category: BasicKeys, label_zh: "Shift+回车", label_en: "Shift+Enter", action: s(vk::RETURN, MOD_SHIFT, "Shift+Enter") },
        PresetAction { id: "copy", category: BasicKeys, label_zh: "复制 Ctrl+C", label_en: "Copy Ctrl+C", action: s(vk::C, MOD_CONTROL, "Ctrl+C") },
        PresetAction { id: "paste", category: BasicKeys, label_zh: "粘贴 Ctrl+V", label_en: "Paste Ctrl+V", action: s(vk::V, MOD_CONTROL, "Ctrl+V") },
        PresetAction { id: "cut", category: BasicKeys, label_zh: "剪切 Ctrl+X", label_en: "Cut Ctrl+X", action: s(vk::X, MOD_CONTROL, "Ctrl+X") },
        PresetAction { id: "select_all", category: BasicKeys, label_zh: "全选 Ctrl+A", label_en: "Select All Ctrl+A", action: s(vk::A, MOD_CONTROL, "Ctrl+A") },
        PresetAction { id: "undo", category: BasicKeys, label_zh: "撤销 Ctrl+Z", label_en: "Undo Ctrl+Z", action: s(vk::Z, MOD_CONTROL, "Ctrl+Z") },
        PresetAction { id: "redo", category: BasicKeys, label_zh: "重做 Ctrl+Y", label_en: "Redo Ctrl+Y", action: s(vk::Y, MOD_CONTROL, "Ctrl+Y") },
        PresetAction { id: "find", category: BasicKeys, label_zh: "查找 Ctrl+F", label_en: "Find Ctrl+F", action: s(vk::F, MOD_CONTROL, "Ctrl+F") },
        PresetAction { id: "save", category: BasicKeys, label_zh: "保存 Ctrl+S", label_en: "Save Ctrl+S", action: s(vk::S, MOD_CONTROL, "Ctrl+S") },
        PresetAction { id: "new_chat", category: BasicKeys, label_zh: "新对话 Ctrl+N", label_en: "New Ctrl+N", action: s(vk::N, MOD_CONTROL, "Ctrl+N") },
        PresetAction { id: "delete", category: BasicKeys, label_zh: "删除 Delete", label_en: "Delete", action: s(vk::DELETE, 0, "Delete") },
        PresetAction { id: "backspace", category: BasicKeys, label_zh: "退格", label_en: "Backspace", action: s(vk::BACK, 0, "Backspace") },
        PresetAction { id: "arrow_up", category: BasicKeys, label_zh: "方向键 上", label_en: "Arrow Up", action: s(vk::UP, 0, "↑") },
        PresetAction { id: "arrow_down", category: BasicKeys, label_zh: "方向键 下", label_en: "Arrow Down", action: s(vk::DOWN, 0, "↓") },
        PresetAction { id: "arrow_left", category: BasicKeys, label_zh: "方向键 左", label_en: "Arrow Left", action: s(vk::LEFT, 0, "←") },
        PresetAction { id: "arrow_right", category: BasicKeys, label_zh: "方向键 右", label_en: "Arrow Right", action: s(vk::RIGHT, 0, "→") },
        PresetAction { id: "browser_back", category: BasicKeys, label_zh: "浏览器后退", label_en: "Browser Back", action: s(vk::BROWSER_BACK, 0, "Back") },
        PresetAction { id: "browser_forward", category: BasicKeys, label_zh: "浏览器前进", label_en: "Browser Forward", action: s(vk::BROWSER_FORWARD, 0, "Forward") },
        PresetAction { id: "page_down", category: BasicKeys, label_zh: "翻页 PgDn", label_en: "Page Down", action: s(vk::NEXT, 0, "PgDn") },
        PresetAction { id: "page_up", category: BasicKeys, label_zh: "上翻 PgUp", label_en: "Page Up", action: s(vk::PRIOR, 0, "PgUp") },
        PresetAction { id: "volume_up", category: SystemAndMedia, label_zh: "音量加", label_en: "Volume Up", action: ButtonAction::VolumeUp },
        PresetAction { id: "volume_down", category: SystemAndMedia, label_zh: "音量减", label_en: "Volume Down", action: ButtonAction::VolumeDown },
        PresetAction { id: "volume_mute", category: SystemAndMedia, label_zh: "静音", label_en: "Mute", action: ButtonAction::VolumeMute },
        PresetAction { id: "play_pause", category: SystemAndMedia, label_zh: "播放/暂停", label_en: "Play/Pause", action: ButtonAction::MediaKey { code: MediaKeyCode::PlayPause } },
        PresetAction { id: "next", category: SystemAndMedia, label_zh: "下一曲", label_en: "Next", action: ButtonAction::MediaKey { code: MediaKeyCode::Next } },
        PresetAction { id: "previous", category: SystemAndMedia, label_zh: "上一曲", label_en: "Previous", action: ButtonAction::MediaKey { code: MediaKeyCode::Previous } },
        PresetAction { id: "show_desktop", category: SystemAndMedia, label_zh: "显示桌面", label_en: "Show Desktop", action: ButtonAction::ShowDesktop },
        PresetAction { id: "task_view", category: SystemAndMedia, label_zh: "任务视图", label_en: "Task View", action: ButtonAction::TaskView },
        PresetAction { id: "app_switcher", category: SystemAndMedia, label_zh: "切换应用", label_en: "Switch App", action: ButtonAction::AppSwitcher },
        PresetAction { id: "screenshot_full", category: SystemAndMedia, label_zh: "截图（全屏）", label_en: "Screenshot (Full)", action: ButtonAction::Screenshot { region: false } },
        PresetAction { id: "screenshot_region", category: SystemAndMedia, label_zh: "截图（区域）", label_en: "Screenshot (Region)", action: ButtonAction::Screenshot { region: true } },
        PresetAction { id: "click_confirm", category: SystemAndMedia, label_zh: "点击确认按钮", label_en: "Click Confirm", action: ButtonAction::ClickConfirm },
        PresetAction { id: "open_settings", category: Custom, label_zh: "打开声桥设置", label_en: "Open SoundBridge", action: ButtonAction::OpenSettings },
    ]
}

impl ButtonAction {
    /// 是否允许按住连发（参考实现：粘贴/复制等一次性动作不连发）。
    pub fn allows_repeat(&self) -> bool {
        match self {
            ButtonAction::Shortcut { vk, .. } => matches!(
                vk,
                0x25 | 0x26 | 0x27 | 0x28 | 0x08 | 0x21 | 0x22 | 0x2D | 0x2E // 方向/退格/翻页/删除
            ),
            ButtonAction::MediaKey { .. } => true,
            ButtonAction::VolumeUp | ButtonAction::VolumeDown => true,
            _ => false,
        }
    }

    pub fn category(&self) -> ActionCategory {
        match self {
            ButtonAction::Disabled
            | ButtonAction::Shortcut { .. }
            | ButtonAction::Custom { .. }
            | ButtonAction::TriggerHandsFree => ActionCategory::BasicKeys,
            ButtonAction::MediaKey { .. }
            | ButtonAction::VolumeUp
            | ButtonAction::VolumeDown
            | ButtonAction::VolumeMute
            | ButtonAction::ShowDesktop
            | ButtonAction::TaskView
            | ButtonAction::AppSwitcher
            | ButtonAction::Screenshot { .. }
            | ButtonAction::ClickConfirm => ActionCategory::SystemAndMedia,
            ButtonAction::OpenApp { .. } | ButtonAction::OpenUrl { .. } => {
                ActionCategory::Applications
            }
            ButtonAction::OpenSettings => ActionCategory::Custom,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_ids_unique() {
        let presets = preset_actions();
        let mut ids: Vec<_> = presets.iter().map(|p| p.id).collect();
        ids.sort();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "duplicate preset ids");
    }

    #[test]
    fn repeat_policy_matches_navigation_keys() {
        assert!(ButtonAction::Shortcut { vk: vk::LEFT, modifiers: 0, label: "←".into() }.allows_repeat());
        assert!(!ButtonAction::Shortcut { vk: vk::C, modifiers: MOD_CONTROL, label: "Ctrl+C".into() }.allows_repeat());
        assert!(!ButtonAction::OpenApp { target: "x".into(), label: "x".into() }.allows_repeat());
    }

    #[test]
    fn action_serializes_stably() {
        let action = ButtonAction::Shortcut { vk: vk::RETURN, modifiers: MOD_CONTROL, label: "Ctrl+Enter".into() };
        let json = serde_json::to_string(&action).unwrap();
        let back: ButtonAction = serde_json::from_str(&json).unwrap();
        assert_eq!(action, back);
    }
}
