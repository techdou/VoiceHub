//! 语音输入 Provider（听写工具）配置与触发策略。
//!
//! SayIt uses embedded PCM capture. External tools retain their virtual-cable
//! and keyboard trigger adapters via the Custom kind (free shortcut + mode).

use serde::{Deserialize, Serialize};

/// 旧版硬编码 Provider（微信输入法 Ctrl+Win / 豆包左 Ctrl hold / Win+H）
/// 的默认键位：settings.rs v2→v3 迁移把旧 kind 改写为 Custom 时预填，
/// 保证升级用户的外部工具触发行为不变。
pub mod legacy_shortcuts {
    /// 微信输入法语音开关：Ctrl+Win（v1.0.3 实测基线）。
    pub const WETYPE_TOGGLE: (u16, u8) = (crate::actions::vk::LWIN, crate::actions::MOD_CONTROL);
    /// 豆包输入法按住式默认占位键：左 Ctrl。
    pub const DOUBAO_HOLD: (u16, u8) = (crate::actions::vk::LCONTROL, 0);
    /// Windows 听写：Win+H。
    pub const WIN_H: (u16, u8) = (crate::actions::vk::H, crate::actions::MOD_WIN);
}

/// 右 Ctrl（SayIt 按住说话默认推荐键）。
pub const VK_RCONTROL: u16 = 0xA3;
/// 右 Alt（SayIt 可选触发键；若 SayIt 的 HF 功能占用右 Alt 需先挪走）。
pub const VK_RMENU: u16 = 0xA5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// SayIt（本地 Whisper 转写）：按住式注入右 Ctrl，
    /// SayIt 录音设备设为 CABLE Output，转写文本落光标处。
    /// 显式 rename："SayIt" 的 snake_case 会得到 "say_it"，与前端字面量 "sayit"
    /// 不一致，曾导致 kind=sayit 的 save_settings 反序列化失败、设置静默不落盘。
    #[serde(rename = "sayit")]
    SayIt,
    /// 自定义快捷键 + 触发模式（外部输入法 / 听写工具的统一接入点）。
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
    /// SayIt 触发键主键（kind == SayIt 时生效；0 = 默认右 Alt）。
    /// 独立字段：与 custom_vk 混用会在 SayIt/Custom 间互相污染。
    pub sayit_vk: u16,
    /// SayIt 触发键修饰键位掩码（MOD_* 组合；0 = 无修饰的单键）。
    /// 单键注入会被 SayIt 的 LL 钩子注入过滤丢弃（is_synthetic，0.1.9 源码
    /// keyboard/mod.rs:1695）；**组合键走 RegisterHotKey 通道、不区分注入**，
    /// 是程序触发 SayIt 免提模式的唯一可行路径（2026-09-07 实测）。
    pub sayit_modifiers: u8,
    /// toggle 模式收尾触发的延迟（毫秒），等待音频排空。
    pub stop_delay_ms: u32,
    /// 麦克风启动等待（毫秒）：给 Provider 留出启动时间，期间音频先缓冲。
    pub startup_grace_ms: u32,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            kind: ProviderKind::SayIt,
            custom_vk: 0,
            custom_modifiers: 0,
            custom_mode: TriggerMode::Toggle,
            sayit_vk: 0,
            sayit_modifiers: 0,
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
    /// 当前生效的 (vk, modifiers)（诊断页也用它展示）。
    pub fn shortcut(&self) -> (u16, u8) {
        match self.kind {
            // 默认右 Alt（与作者实际 SayIt 配置对齐）；用户可在 UI 换右 Ctrl
            // 或组合键。避免右 Shift——长按 8s 触发筛选键会让录音停不下来。
            // 注意：单键（modifiers=0）注入会被 SayIt 的钩子注入过滤丢弃，
            // 程序联动必须用组合键（RegisterHotKey 通道）。
            ProviderKind::SayIt => {
                (if self.sayit_vk != 0 { self.sayit_vk } else { VK_RMENU }, self.sayit_modifiers)
            }
            ProviderKind::Custom => (self.custom_vk, self.custom_modifiers),
            ProviderKind::None => (0, 0),
        }
    }

    fn mode(&self) -> Option<TriggerMode> {
        match self.kind {
            // SayIt 双模式：Toggle = 免提（HF，点一下开始/再点结束），
            // Hold = 按住说话（PTT）。跟随用户选择，默认 Toggle。
            ProviderKind::SayIt => None,
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
            ProviderKind::None | ProviderKind::SayIt => 0,
            _ => self.stop_delay_ms.max(120),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{MOD_ALT, MOD_CONTROL};

    #[test]
    fn provider_kind_wire_names_match_frontend_literals() {
        // 前端 types.ts 的字面量联合；内嵌大写缩写（SayIt）经 snake_case 会变成
        // "say_it" 与前端 "sayit" 脱节，曾致 save_settings 反序列化静默失败。
        let expected = [
            (ProviderKind::SayIt, "sayit"),
            (ProviderKind::Custom, "custom"),
            (ProviderKind::None, "none"),
        ];
        for (kind, wire) in expected {
            assert_eq!(serde_json::to_string(&kind).unwrap(), format!("\"{wire}\""));
            assert_eq!(serde_json::from_str::<ProviderKind>(&format!("\"{wire}\"")).unwrap(), kind);
        }
    }

    #[test]
    fn legacy_provider_kinds_fail_deserialization() {
        // we_type / doubao / win_h 已裁剪：新配置不可能再写出这些 kind，
        // 反序列化必须失败（旧值由 settings.rs v2→v3 迁移改写为 custom），
        // 而不是静默落进错误分支。
        for wire in ["we_type", "doubao", "win_h"] {
            assert!(serde_json::from_str::<ProviderKind>(&format!("\"{wire}\"")).is_err());
        }
    }

    #[test]
    fn legacy_shortcuts_match_removed_providers_defaults() {
        // 迁移预填值的锚点：与被删 Provider 的原默认键位一致，
        // 升级用户的触发行为不因裁剪改变。
        assert_eq!(legacy_shortcuts::WETYPE_TOGGLE, (crate::actions::vk::LWIN, MOD_CONTROL));
        assert_eq!(legacy_shortcuts::DOUBAO_HOLD, (crate::actions::vk::LCONTROL, 0));
        assert_eq!(legacy_shortcuts::WIN_H, (crate::actions::vk::H, crate::actions::MOD_WIN));
    }

    #[test]
    fn none_provider_never_triggers() {
        let config = ProviderConfig { kind: ProviderKind::None, ..Default::default() };
        assert_eq!(config.trigger_on_stream_start(), ProviderTrigger::None);
        assert_eq!(config.trigger_on_stream_stop(), ProviderTrigger::None);
        assert_eq!(config.drain_ms(), 0);
    }

    #[test]
    fn embedded_sayit_never_injects_keys_for_either_legacy_mode() {
        let config = ProviderConfig { kind: ProviderKind::SayIt, ..Default::default() };
        // 默认免提模式（HF）：开始/结束各点一下。
        assert_eq!(
            config.trigger_on_stream_start(),
            ProviderTrigger::None
        );
        assert_eq!(
            config.trigger_on_stream_stop(),
            ProviderTrigger::None
        );
        // 按住说话（PTT）：按下并保持、结束时释放。
        let ptt = ProviderConfig {
            kind: ProviderKind::SayIt,
            custom_mode: TriggerMode::Hold,
            ..Default::default()
        };
        assert_eq!(
            ptt.trigger_on_stream_start(),
            ProviderTrigger::None
        );
        assert_eq!(
            ptt.trigger_on_stream_stop(),
            ProviderTrigger::None
        );
        // 用户改键（如右 Ctrl）后生效。
        let ctrl = ProviderConfig {
            kind: ProviderKind::SayIt,
            sayit_vk: VK_RCONTROL,
            ..Default::default()
        };
        assert_eq!(
            ctrl.trigger_on_stream_start(),
            ProviderTrigger::None
        );
        assert_eq!(config.drain_ms(), 0);
    }

    #[test]
    fn embedded_sayit_ignores_legacy_combo_and_loads_old_settings() {
        // 组合键（Ctrl+Alt+H）：SayIt 的 RegisterHotKey 通道不区分注入，
        // 是程序触发免提模式的唯一可行路径（2026-09-07 实测）。
        let combo = ProviderConfig {
            kind: ProviderKind::SayIt,
            sayit_vk: 0x48,
            sayit_modifiers: MOD_CONTROL | MOD_ALT,
            ..Default::default()
        };
        assert_eq!(
            combo.trigger_on_stream_start(),
            ProviderTrigger::None
        );
        // 反序列化兼容：旧配置无 sayitModifiers 字段 → 默认 0（单键右 Alt）。
        let legacy = serde_json::from_str::<ProviderConfig>(
            r#"{"kind":"sayit","customVk":0,"customModifiers":0,"customMode":"toggle","sayitVk":0,"stopDelayMs":180,"startupGraceMs":80}"#,
        )
        .unwrap();
        assert_eq!(legacy.sayit_modifiers, 0);
        assert_eq!(legacy.shortcut(), (VK_RMENU, 0));
    }

    #[test]
    fn sayit_and_custom_keys_do_not_cross_contaminate() {
        // 回归：sayit_vk 与 custom_vk 必须互不影响（M6 事故的根因是
        // 两者共用一个字段，SayIt 里改键后 Custom 静默继承）。
        let custom = ProviderConfig {
            kind: ProviderKind::Custom,
            custom_vk: 0x4B,
            custom_modifiers: MOD_CONTROL,
            custom_mode: TriggerMode::Toggle,
            sayit_vk: VK_RCONTROL,
            ..Default::default()
        };
        assert_eq!(
            custom.trigger_on_stream_start(),
            ProviderTrigger::Tap { vk: 0x4B, modifiers: MOD_CONTROL }
        );
        let sayit = ProviderConfig {
            kind: ProviderKind::SayIt,
            custom_vk: 0x4B,
            custom_modifiers: MOD_CONTROL,
            ..Default::default()
        };
        // SayIt 无视 custom_vk：sayit_vk 未设 → 默认右 Alt。
        assert_eq!(
            sayit.trigger_on_stream_start(),
            ProviderTrigger::None
        );
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
        let config = ProviderConfig { kind: ProviderKind::Custom, stop_delay_ms: 10, ..Default::default() };
        assert_eq!(config.drain_ms(), 120);
    }
}
