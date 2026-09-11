//! 遥控器型号识别（BLE 2A24 Model Number）。

use serde::{Deserialize, Serialize};

use crate::buttons::RemoteButton;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteModel {
    /// 小米蓝牙遥控器 2（RC001）：8 kHz 档固件行为更保守。
    Rc001,
    /// 小米蓝牙遥控器 2 Pro（RC003）/ MI RC。
    Rc003,
    /// ARN9 固件：ADPCM 低半字节优先。
    Arn9,
    /// 未识别（按默认行为运行）。
    Unknown,
}

impl RemoteModel {
    /// 依据 2A24 型号字符串识别。匹配规则参考实测记录：
    /// RC001 / RC003 直接包含型号；ARN9 固件在型号或相邻字段中暴露代号。
    pub fn identify(model_number: &str) -> Self {
        let normalized = model_number.trim().to_uppercase();
        if normalized.contains("ARN9") {
            return RemoteModel::Arn9;
        }
        if normalized.contains("RC003") || normalized.contains("MI RC") {
            return RemoteModel::Rc003;
        }
        if normalized.contains("RC001") {
            return RemoteModel::Rc001;
        }
        RemoteModel::Unknown
    }

    /// ARN9 固件的 ADPCM nibble 顺序为低半字节优先。
    pub fn adpcm_low_nibble_first(self) -> bool {
        matches!(self, RemoteModel::Arn9)
    }

    pub fn display_name(self) -> &'static str {
        match self {
            RemoteModel::Rc001 => "小米蓝牙遥控器 2（RC001）",
            RemoteModel::Rc003 => "小米蓝牙遥控器 2 Pro（RC003）",
            RemoteModel::Arn9 => "小米遥控器（ARN9 固件）",
            RemoteModel::Unknown => "未识别型号",
        }
    }

    /// 该型号机身上不存在的物理键（映射页据此置灰，避免"配了白配"）。
    /// 实测基线（RC003，PID 0x5070）：五接口 HID 地图里无电源 / 返回 /
    /// TV 报文——触摸板导航（Up/Down/Ok）、音量键（consumer 页）与语音键
    /// （键盘页 F5）实测可用；Home / 菜单的通道待采集（probe_hid）。
    /// 未实测型号返回空集：宁可多显示，不臆断缺失。
    pub fn absent_buttons(self) -> &'static [RemoteButton] {
        match self {
            RemoteModel::Rc003 => {
                &[RemoteButton::Power, RemoteButton::Back, RemoteButton::Tv]
            }
            RemoteModel::Rc001 | RemoteModel::Arn9 | RemoteModel::Unknown => &[],
        }
    }
}

/// 设备名匹配（扫描广告时用）：小米遥控器的广播名。
/// 中文命名存在多个变体（"小米遥控器"、"小米蓝牙语音遥控器"），统一以"遥控器"子串兜底。
pub fn is_voice_remote_name(name: Option<&str>) -> bool {
    let Some(name) = name else { return false };
    let lower = name.trim().to_lowercase();
    lower.contains("mi rc")
        || lower.contains("rc003")
        || lower.contains("rc001")
        || lower.contains("遥控器")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_models() {
        assert_eq!(RemoteModel::identify("Mi Remote Control 2 Pro RC003"), RemoteModel::Rc003);
        assert_eq!(RemoteModel::identify("rc001"), RemoteModel::Rc001);
        assert_eq!(RemoteModel::identify("ARN9 v2"), RemoteModel::Arn9);
        assert_eq!(RemoteModel::identify("random"), RemoteModel::Unknown);
        assert_eq!(RemoteModel::identify("  MI RC  "), RemoteModel::Rc003);
    }

    #[test]
    fn arn9_uses_low_nibble_first() {
        assert!(RemoteModel::Arn9.adpcm_low_nibble_first());
        assert!(!RemoteModel::Rc003.adpcm_low_nibble_first());
    }

    #[test]
    fn rc003_marks_unimplemented_physical_buttons_absent() {
        let absent = RemoteModel::Rc003.absent_buttons();
        assert!(absent.contains(&RemoteButton::Power));
        assert!(absent.contains(&RemoteButton::Back));
        assert!(absent.contains(&RemoteButton::Tv));
        // 实测可用的键不在缺失集。
        for present in [RemoteButton::Ok, RemoteButton::Up, RemoteButton::Down, RemoteButton::VolumeUp, RemoteButton::VolumeDown] {
            assert!(!absent.contains(&present));
        }
    }

    #[test]
    fn unmeasured_models_claim_no_absent_buttons() {
        // RC001 / ARN9 未做键位实测：必须返回空集（宁可多显示不臆断）。
        assert!(RemoteModel::Rc001.absent_buttons().is_empty());
        assert!(RemoteModel::Arn9.absent_buttons().is_empty());
        assert!(RemoteModel::Unknown.absent_buttons().is_empty());
    }

    #[test]
    fn matches_advertised_names() {
        assert!(is_voice_remote_name(Some("Mi RC Pro")));
        assert!(is_voice_remote_name(Some("RC003")));
        // 中文系统配对名实为"小米蓝牙语音遥控器"——"小米遥控器"整词匹配漏掉，按"遥控器"兜底。
        assert!(is_voice_remote_name(Some("小米蓝牙语音遥控器")));
        assert!(is_voice_remote_name(Some("小米遥控器")));
        assert!(!is_voice_remote_name(Some("JBL Flip 6")));
        assert!(!is_voice_remote_name(Some("Mi Mouse3C")));
        assert!(!is_voice_remote_name(None));
    }
}
