//! 按键映射：每个按键 → { 单击 / 双击 / 长按 } 三槽动作。
//!
//! 语音键不参与映射（固定为开麦）。支持双击/长按的按键见
//! `RemoteButton::supports_secondary`，其余按键双击/长按槽忽略。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::actions::ButtonAction;
use crate::buttons::RemoteButton;
use crate::gesture::Gesture;

/// 单个按键的三个动作槽。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ButtonBinding {
    pub single: ButtonAction,
    pub double: ButtonAction,
    pub long: ButtonAction,
}

impl Default for ButtonBinding {
    fn default() -> Self {
        Self {
            single: ButtonAction::Disabled,
            double: ButtonAction::Disabled,
            long: ButtonAction::Disabled,
        }
    }
}

impl ButtonBinding {
    pub fn single(action: ButtonAction) -> Self {
        Self { single: action, ..Default::default() }
    }
}

/// 一整套按键映射（按键 ID → 绑定）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ButtonMapping {
    pub bindings: HashMap<String, ButtonBinding>,
}

impl ButtonMapping {
    pub fn key(button: RemoteButton) -> String {
        serde_json::to_value(button)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| format!("{button:?}").to_lowercase())
    }

    pub fn get(&self, button: RemoteButton) -> ButtonBinding {
        self.bindings.get(&Self::key(button)).cloned().unwrap_or_default()
    }

    pub fn set(&mut self, button: RemoteButton, binding: ButtonBinding) {
        let key = Self::key(button);
        if binding == ButtonBinding::default() {
            self.bindings.remove(&key);
        } else {
            self.bindings.insert(key, binding);
        }
    }

    /// 手势 → 动作解析。不支持二级手势的按键上，双击/长按一律落到单击槽
    /// （或 Disabled），避免手势识别出 DoubleClick 却查到空动作。
    pub fn resolve(&self, button: RemoteButton, gesture: Gesture) -> Option<ButtonAction> {
        let binding = self.get(button);
        let action = match gesture {
            Gesture::SingleClick | Gesture::Repeat => binding.single,
            Gesture::DoubleClick => {
                if button.supports_secondary() {
                    binding.double
                } else {
                    ButtonAction::Disabled
                }
            }
            Gesture::LongPress => {
                if button.supports_secondary() {
                    binding.long
                } else {
                    ButtonAction::Disabled
                }
            }
        };
        match action {
            ButtonAction::Disabled => None,
            other => Some(other),
        }
    }
}

/// 出厂默认映射：导航键位 + 常用编辑组合（对齐 vibe-flow“通用导航”预设）。
pub fn default_mapping() -> ButtonMapping {
    use crate::actions::vk;
    let shortcut = |vk: u16, m: u8, label: &str| {
        ButtonAction::Shortcut { vk, modifiers: m, label: label.into() }
    };
    let mut mapping = ButtonMapping::default();
    mapping.set(
        RemoteButton::Up,
        ButtonBinding::single(shortcut(vk::UP, 0, "↑")),
    );
    mapping.set(
        RemoteButton::Down,
        ButtonBinding::single(shortcut(vk::DOWN, 0, "↓")),
    );
    mapping.set(
        RemoteButton::Left,
        ButtonBinding::single(shortcut(vk::LEFT, 0, "←")),
    );
    mapping.set(
        RemoteButton::Right,
        ButtonBinding::single(shortcut(vk::RIGHT, 0, "→")),
    );
    mapping.set(
        RemoteButton::Ok,
        ButtonBinding::single(shortcut(vk::RETURN, 0, "Enter")),
    );
    mapping.set(
        RemoteButton::Back,
        ButtonBinding::single(shortcut(vk::BROWSER_BACK, 0, "Back")),
    );
    mapping.set(RemoteButton::Home, ButtonBinding::default());
    mapping.set(RemoteButton::VolumeUp, ButtonBinding::single(ButtonAction::VolumeUp));
    mapping.set(RemoteButton::VolumeDown, ButtonBinding::single(ButtonAction::VolumeDown));
    mapping
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{ButtonAction, CustomShortcut, MOD_CONTROL};

    fn paste() -> ButtonAction {
        ButtonAction::Custom {
            shortcut: CustomShortcut::new(0x56, MOD_CONTROL, "Ctrl+V"),
        }
    }

    #[test]
    fn resolve_single_action() {
        let m = default_mapping();
        let action = m.resolve(RemoteButton::Up, Gesture::SingleClick).unwrap();
        assert_eq!(
            action,
            ButtonAction::Shortcut { vk: 0x26, modifiers: 0, label: "↑".into() }
        );
    }

    #[test]
    fn secondary_slots_only_for_supported_buttons() {
        let mut m = ButtonMapping::default();
        let mut binding = ButtonBinding::default();
        binding.single = paste();
        binding.double = ButtonAction::ShowDesktop;
        m.set(RemoteButton::Home, binding.clone());
        // Home 支持二级 → 双击命中 double 槽。
        assert_eq!(m.resolve(RemoteButton::Home, Gesture::DoubleClick), Some(ButtonAction::ShowDesktop));

        m.set(RemoteButton::Up, binding);
        // Up 不支持二级 → 双击/长按为空。
        assert_eq!(m.resolve(RemoteButton::Up, Gesture::DoubleClick), None);
        assert_eq!(m.resolve(RemoteButton::Up, Gesture::LongPress), None);
        assert!(m.resolve(RemoteButton::Up, Gesture::SingleClick).is_some());
    }

    #[test]
    fn repeat_falls_back_to_single() {
        let mut m = ButtonMapping::default();
        let mut b = ButtonBinding::default();
        b.single = ButtonAction::VolumeDown;
        m.set(RemoteButton::VolumeDown, b);
        assert_eq!(m.resolve(RemoteButton::VolumeDown, Gesture::Repeat), Some(ButtonAction::VolumeDown));
    }

    #[test]
    fn unset_binding_resolves_to_none() {
        let m = ButtonMapping::default();
        assert_eq!(m.resolve(RemoteButton::Menu, Gesture::SingleClick), None);
    }

    #[test]
    fn default_binding_serialization_roundtrip() {
        let m = default_mapping();
        let json = serde_json::to_string(&m).unwrap();
        let back: ButtonMapping = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn setting_default_binding_removes_entry() {
        let mut m = default_mapping();
        m.set(RemoteButton::Menu, ButtonBinding::default());
        assert!(!m.bindings.contains_key(&ButtonMapping::key(RemoteButton::Menu)));
    }
}
