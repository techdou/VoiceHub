//! 语音输入 Provider（听写工具）配置与触发策略。
//!
//! 声桥不做语音识别：解码音频 → 播入 VB-CABLE 输入端；Provider
//! 在 CABLE 输出端收音、转文字、写进聚焦输入框。本模块只负责
//! “会话开始/结束时对 Provider 发什么触发键”的纯逻辑。

use serde::{Deserialize, Serialize};

use crate::actions::vk;

/// 微信输入法默认语音开关：Ctrl+Win（v1.0.3 实测基线）。
pub const WETYPE_TOGGLE: (u16, u8) = (vk::LWIN, crate::actions::MOD_CONTROL);
/// Win+H：Windows 内置听写。
pub const WIN_H: (u16, u8) = (vk::H, crate::actions::MOD_WIN);
/// 右 Ctrl（SayIt 按住说话默认推荐键）。
pub const VK_RCONTROL: u16 = 0xA3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// 微信输入法：toggle 触发，开麦时触发一次，松开后延迟再触发一次收尾。
    WeType,
    /// 豆包输入法：按住式（hold）——开麦时按下快捷键，结束时释放。
    Doubao,
    /// Windows 听写 Win+H。
    WinH,
    /// SayIt（本地 Whisper 转写）：按住式注入右 Ctrl，
    /// SayIt 录音设备设为 CABLE Output，转写文本落光标处。
    SayIt,
    /// 自定义快捷键 + 触发模式。
    Custom,
    /// 仅输出音频，不触发任何 Provider（自证音频链路用）。
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerMode {
    /// 一次按键切换开始/结束。
    Toggle,
    /// 按住说话：开始时按下、结束时释放。
    Hold,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    /// 自定义快捷键（kind == Custom 时生效）。
    pub custom_vk: u16,
    pub custom_modifiers: u8,
    pub custom_mode: TriggerMode,
    /// toggle 模式收尾触发的延迟（毫秒），等待音频排空。
    pub stop_delay_ms: u32,
    /// 麦克风启动等待（毫秒）：给 Provider 留出启动时间，期间音频先缓冲。
    pub startup_grace_ms: u32,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            kind: ProviderKind::WeType,
            custom_vk: 0,
            custom_modifiers: 0,
            custom_mode: TriggerMode::Toggle,
            stop_delay_ms: 180,
            startup_grace_ms: 80,
        }
    }
}

/// 触发命令（宿主翻译成 SendInput）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderTrigger {
    /// 按下并立即释放组合键。
    Tap { vk: u16, modifiers: u8 },
    /// 按下组合键并保持（hold 模式开始）。
    Press { vk: u16, modifiers: u8 },
    /// 释放 hold 模式的组合键（会话结束）。
    Release { vk: u16, modifiers: u8 },
    /// 无动作。
    None,
}

impl ProviderConfig {
    fn shortcut(&self) -> (u16, u8) {
        match self.kind {
            ProviderKind::WeType => WETYPE_TOGGLE,
            ProviderKind::Doubao => (vk::LCONTROL, 0), // 默认占位：豆包按住式默认键可在 UI 改
            ProviderKind::WinH => WIN_H,
            // 右 Ctrl：扩展键（扫描码 0x1D + EXTENDED）。避免右 Shift——
            // 长按 8s 触发 Windows 筛选键会让录音停不下来（SayIt 官方提示）。
            ProviderKind::SayIt => (VK_RCONTROL, 0),
            ProviderKind::Custom => (self.custom_vk, self.custom_modifiers),
            ProviderKind::None => (0, 0),
        }
    }

    fn mode(&self) -> Option<TriggerMode> {
        match self.kind {
            ProviderKind::WeType => Some(TriggerMode::Toggle),
            ProviderKind::Doubao => Some(TriggerMode::Hold),
            ProviderKind::WinH => Some(TriggerMode::Toggle),
            ProviderKind::SayIt => Some(TriggerMode::Hold),
            ProviderKind::Custom => Some(self.custom_mode),
            ProviderKind::None => None,
        }
    }

    /// 语音流开始时对 Provider 的触发。
    pub fn trigger_on_stream_start(&self) -> ProviderTrigger {
        let (vk, modifiers) = self.shortcut();
        if vk == 0 {
            return ProviderTrigger::None;
        }
        match self.mode() {
            Some(TriggerMode::Hold) => ProviderTrigger::Press { vk, modifiers },
            Some(TriggerMode::Toggle) => ProviderTrigger::Tap { vk, modifiers },
            None => ProviderTrigger::None,
        }
    }

    /// 语音流结束时对 Provider 的触发（宿主应先等 stop_delay_ms 排空音频）。
    pub fn trigger_on_stream_stop(&self) -> ProviderTrigger {
        let (vk, modifiers) = self.shortcut();
        if vk == 0 {
            return ProviderTrigger::None;
        }
        match self.mode() {
            Some(TriggerMode::Hold) => ProviderTrigger::Release { vk, modifiers },
            // toggle 模式：结束时再敲一次让 Provider 收尾转写。
            Some(TriggerMode::Toggle) => ProviderTrigger::Tap { vk, modifiers },
            None => ProviderTrigger::None,
        }
    }

    /// 音频排空等待（毫秒）。Provider 在 CABLE 输出端收音，
    /// 排空等待 = 尾音 + 管线缓冲。
    pub fn drain_ms(&self) -> u32 {
        match self.kind {
            ProviderKind::None => 0,
            _ => self.stop_delay_ms.max(120),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::MOD_CONTROL;

    #[test]
    fn wetype_toggle_taps_on_both_edges() {
        let config = ProviderConfig::default();
        assert_eq!(
            config.trigger_on_stream_start(),
            ProviderTrigger::Tap { vk: vk::LWIN, modifiers: MOD_CONTROL }
        );
        assert_eq!(
            config.trigger_on_stream_stop(),
            ProviderTrigger::Tap { vk: vk::LWIN, modifiers: MOD_CONTROL }
        );
    }

    #[test]
    fn doubao_hold_presses_then_releases() {
        let config = ProviderConfig { kind: ProviderKind::Doubao, ..Default::default() };
        assert!(matches!(config.trigger_on_stream_start(), ProviderTrigger::Press { .. }));
        assert!(matches!(config.trigger_on_stream_stop(), ProviderTrigger::Release { .. }));
    }

    #[test]
    fn win_h_taps() {
        let config = ProviderConfig { kind: ProviderKind::WinH, ..Default::default() };
        assert_eq!(
            config.trigger_on_stream_start(),
            ProviderTrigger::Tap { vk: vk::H, modifiers: crate::actions::MOD_WIN }
        );
    }

    #[test]
    fn none_provider_never_triggers() {
        let config = ProviderConfig { kind: ProviderKind::None, ..Default::default() };
        assert_eq!(config.trigger_on_stream_start(), ProviderTrigger::None);
        assert_eq!(config.trigger_on_stream_stop(), ProviderTrigger::None);
        assert_eq!(config.drain_ms(), 0);
    }

    #[test]
    fn sayit_holds_right_control() {
        let config = ProviderConfig { kind: ProviderKind::SayIt, ..Default::default() };
        assert_eq!(
            config.trigger_on_stream_start(),
            ProviderTrigger::Press { vk: VK_RCONTROL, modifiers: 0 }
        );
        assert_eq!(
            config.trigger_on_stream_stop(),
            ProviderTrigger::Release { vk: VK_RCONTROL, modifiers: 0 }
        );
        // 排空等待合理（Whisper 录音收尾）。
        assert!(config.drain_ms() >= 120);
    }

    #[test]
    fn custom_provider_uses_custom_shortcut() {
        let config = ProviderConfig {
            kind: ProviderKind::Custom,
            custom_vk: 0x4B,
            custom_modifiers: MOD_CONTROL,
            custom_mode: TriggerMode::Hold,
            ..Default::default()
        };
        assert_eq!(
            config.trigger_on_stream_start(),
            ProviderTrigger::Press { vk: 0x4B, modifiers: MOD_CONTROL }
        );
        assert_eq!(
            config.trigger_on_stream_stop(),
            ProviderTrigger::Release { vk: 0x4B, modifiers: MOD_CONTROL }
        );
    }

    #[test]
    fn drain_has_minimum() {
        let config = ProviderConfig { stop_delay_ms: 10, ..Default::default() };
        assert_eq!(config.drain_ms(), 120);
    }
}
