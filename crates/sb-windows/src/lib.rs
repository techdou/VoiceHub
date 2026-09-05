//! SoundBridge Windows 平台层。
//!
//! 模块划分：
//! - `send_input`：按键注入（快捷键 / 媒体键 / 音量）
//! - `raw_input`：遥控器 HID 捕获（Xiaomi VID 过滤；语音键 F5 + usage 数组报文）
//! - `key_gate`：低级键盘钩子，吞掉遥控器原生按键避免穿透
//! - `audio`：WASAPI 输出到指定端点（VB-CABLE）
//! - `foreground`：前台进程名（Smart Profiles 匹配）
//! - `power`：系统睡眠 / 唤醒通知
//! - `radio`：蓝牙无线电自愈（僵死链路恢复）
//! - `ble`：WinRT BLE 连接与 ATVV 语音会话

#[cfg(windows)]
pub mod audio;
#[cfg(windows)]
pub mod ble;
#[cfg(windows)]
pub mod foreground;
#[cfg(windows)]
pub mod key_gate;
#[cfg(windows)]
pub mod power;
#[cfg(windows)]
pub mod radio;
#[cfg(windows)]
pub mod raw_input;
#[cfg(windows)]
pub mod send_input;
#[cfg(windows)]
pub mod shell;

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("{0}")]
    Message(String),
    #[cfg(windows)]
    #[error("windows api: {0}")]
    Windows(#[from] windows::core::Error),
}

pub type Result<T> = std::result::Result<T, PlatformError>;

/// 输出音频端点描述。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioEndpoint {
    pub id: String,
    pub name: String,
    /// 名称命中 VB-CABLE / 虚拟声卡特征。
    pub is_virtual_cable_candidate: bool,
}

/// 端点名是否为虚拟声卡输入端（我们向它播放音频）。
pub fn is_virtual_cable_input_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("cable input")
        || lower.contains("vb-audio virtual cable")
        || lower.contains("voice meter input")
        || lower.contains("virtual audio cable input")
}

/// 供 Provider 录音的虚拟声卡输出端（提示用户选作录音设备）。
pub fn is_virtual_cable_output_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("cable output") || lower.contains("vb-audio virtual cable")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_virtual_cable_names() {
        assert!(is_virtual_cable_input_name("CABLE Input (VB-Audio Virtual Cable)"));
        assert!(is_virtual_cable_output_name("CABLE Output (VB-Audio Virtual Cable)"));
        assert!(!is_virtual_cable_input_name("扬声器 (Realtek High Definition Audio)"));
    }
}
