//! 遥控器按键模型与 HID 报文解析。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

static USAGE_MAP_CELL: OnceLock<HashMap<u16, RemoteButton>> = OnceLock::new();

fn usage_map() -> &'static HashMap<u16, RemoteButton> {
    USAGE_MAP_CELL.get_or_init(|| {
        RemoteButton::ALL
            .iter()
            .map(|&b| (b.hid_usage(), b))
            .collect()
    })
}

/// RC003 可映射按键（2026-09-27 真机重采定案，9 键）+ 语音键。
/// 电源/TV/左/右与 Home/菜单一样走经典蓝牙键盘页 VK（raw_input.rs 合成），
/// `hid_usage` 是内部合成码而非真机 consumer 报文值——沿用社区交叉核对表
/// （Issue #1 osagem fork）保持一致。返回/音量±在 Windows 上用户态不可达
/// （Raw Input 零报文零系统反应；读原始通道需注入驱动宿主，2026-09-27
/// 评审决定不引入），不入模型。语音键走键盘页 F5，不参与映射。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteButton {
    Up,
    Ok,
    Down,
    Home,
    Menu,
    Left,
    Right,
    Power,
    Tv,
}

impl RemoteButton {
    pub const ALL: [RemoteButton; 9] = [
        RemoteButton::Up,
        RemoteButton::Ok,
        RemoteButton::Down,
        RemoteButton::Home,
        RemoteButton::Menu,
        RemoteButton::Left,
        RemoteButton::Right,
        RemoteButton::Power,
        RemoteButton::Tv,
    ];

    pub fn hid_usage(self) -> u16 {
        match self {
            RemoteButton::Up => 0x52,
            RemoteButton::Ok => 0x28,
            RemoteButton::Down => 0x51,
            RemoteButton::Home => 0x4A,
            RemoteButton::Menu => 0x65,
            RemoteButton::Left => 0x50,
            RemoteButton::Right => 0x4F,
            RemoteButton::Power => 0x66,
            RemoteButton::Tv => 0x35,
        }
    }

    pub fn from_hid_usage(usage: u16) -> Option<Self> {
        usage_map().get(&usage).copied()
    }

    /// 支持双击 / 长按二级动作的按键（其余只有单击）。
    pub fn supports_secondary(self) -> bool {
        matches!(
            self,
            RemoteButton::Home
                | RemoteButton::Menu
                | RemoteButton::Ok
        )
    }
}

/// 解析遥控器 usage 数组报文。
///
/// 已知两种带 report ID 的格式（奇数长度 = 前缀 1 字节 report ID）：
/// - RC001 / 旧代固件：7 字节 `[1, u16×3]`；
/// - 紧凑格式：3 字节 `[2, usage_lo, usage_hi]`。
/// 报文格式不用于识别设备，调用方必须先核对遥控器来源。
/// 返回当前按下的 usage 集合，空集 = 全部释放。
pub fn parse_usage_report(data: &[u8]) -> Option<Vec<u16>> {
    let mut bytes = data;
    // 奇数长度一律剥掉首字节 report ID（覆盖 [2,lo,hi] 与 [1,...×6]）。
    if !bytes.len().is_multiple_of(2) {
        bytes = &bytes[1..];
    }
    if bytes.is_empty() {
        return Some(Vec::new());
    }
    let mut usages = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks(2) {
        let usage = u16::from_le_bytes([pair[0], pair[1]]);
        if usage != 0 {
            usages.push(usage);
        }
    }
    Some(usages)
}

/// 把两次报文的 usage 集合转成按键沿事件（按下 / 释放）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ButtonEdge {
    pub button: RemoteButton,
    pub pressed: bool,
}

pub fn diff_usage_sets(
    previous: &[u16],
    current: &[u16],
) -> Vec<ButtonEdge> {
    let mut edges = Vec::new();
    for &usage in current {
        if !previous.contains(&usage) {
            if let Some(button) = RemoteButton::from_hid_usage(usage) {
                edges.push(ButtonEdge { button, pressed: true });
            }
        }
    }
    for &usage in previous {
        if !current.contains(&usage) {
            if let Some(button) = RemoteButton::from_hid_usage(usage) {
                edges.push(ButtonEdge { button, pressed: false });
            }
        }
    }
    edges
}

/// 语音键：键盘页 usage 0x3E（F5）。Xiaomi VID 0x2717。
/// 本机蓝牙遥控器报 PID 0x32B8；USB PID 0x5070 是鼠标，不能作为遥控器标识。
pub struct VoiceKeyHid;

impl VoiceKeyHid {
    pub const KEYBOARD_USAGE: u16 = 0x3E;
    pub const VENDOR_ID: u16 = 0x2717;
    pub const PRODUCT_ID_RC001: u16 = 0x32B8;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_roundtrip_for_all_buttons() {
        for button in RemoteButton::ALL {
            assert_eq!(RemoteButton::from_hid_usage(button.hid_usage()), Some(button));
        }
    }

    #[test]
    fn parses_array_report_with_leading_report_id() {
        let report = [0x01, 0x52, 0x00, 0x28, 0x00, 0x00, 0x00];
        assert_eq!(parse_usage_report(&report), Some(vec![0x52, 0x28]));
    }

    /// 紧凑 usage 数组格式：3 字节 [reportID=2, usage_lo, usage_hi]。
    #[test]
    fn parses_rc003_three_byte_report() {
        assert_eq!(parse_usage_report(&[0x02, 0xF1, 0x00]), Some(vec![0xF1]));
        assert_eq!(parse_usage_report(&[0x02, 0x52, 0x00]), Some(vec![0x52]));
        // 释放报文：usage 0 → 空集。
        assert_eq!(parse_usage_report(&[0x02, 0x00, 0x00]), Some(vec![]));
    }

    #[test]
    fn parses_bare_array_report() {
        let report = [0xF1, 0x00];
        assert_eq!(parse_usage_report(&report), Some(vec![0xF1]));
    }

    #[test]
    fn empty_report_means_all_released() {
        assert_eq!(parse_usage_report(&[0x00, 0x00]), Some(vec![]));
    }

    #[test]
    fn diff_detects_press_and_release() {
        let prev = vec![0x52u16, 0x28];
        let curr = vec![0x28u16, 0x51];
        let edges = diff_usage_sets(&prev, &curr);
        assert!(edges.contains(&ButtonEdge { button: RemoteButton::Up, pressed: false }));
        assert!(edges.contains(&ButtonEdge { button: RemoteButton::Down, pressed: true }));
        assert_eq!(edges.len(), 2);
    }

    #[test]
    fn secondary_buttons_subset() {
        assert!(RemoteButton::Home.supports_secondary());
        assert!(RemoteButton::Ok.supports_secondary());
        assert!(RemoteButton::Menu.supports_secondary());
        assert!(!RemoteButton::Up.supports_secondary());
    }
}
