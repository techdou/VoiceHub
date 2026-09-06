//! 应用设置模型（版本化 + 迁移）。
//!
//! 存储由宿主负责（Tauri app_data_dir 下的 settings.json）；
//! 本模块只做结构、默认值与 v1 → 当前 的迁移。

use serde::{Deserialize, Serialize};

use crate::profiles::ProfileStore;
use crate::provider::ProviderConfig;

pub const SETTINGS_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub schema_version: u32,
    /// 是否已完成首次引导。
    pub onboarding_complete: bool,
    /// 已配对遥控器的 WinRT DeviceInformation Id（"BluetoothLE#…"）。
    pub paired_device_id: Option<String>,
    pub paired_device_name: Option<String>,
    /// 音频输出端点友好名（VB-CABLE Input 等）。
    pub audio_endpoint_name: String,
    /// 增益（dB，±24）。
    pub gain_db: f64,
    pub provider: ProviderConfig,
    pub profiles: ProfileStore,
    /// 按键自定义映射总开关（关闭 = 遥控器按键直通系统）。
    pub button_mapping_enabled: bool,
    /// 实验性：语音会话每 40s 发送 ATVV 续租（MIC_EXTEND），
    /// 尝试突破约 60s 固件会话边界。固件是否接受未真机验证。
    pub experimental_voice_extend: bool,
    pub launch_at_login: bool,
    pub language: Language,
    pub theme: Theme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    System,
    ZhCn,
    English,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    System,
    Light,
    Dark,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_VERSION,
            onboarding_complete: false,
            paired_device_id: None,
            paired_device_name: None,
            audio_endpoint_name: String::new(),
            gain_db: 0.0,
            provider: ProviderConfig::default(),
            profiles: ProfileStore::default(),
            button_mapping_enabled: true,
            experimental_voice_extend: false,
            launch_at_login: false,
            language: Language::System,
            theme: Theme::System,
        }
    }
}

impl Default for Language {
    fn default() -> Self {
        Language::System
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::System
    }
}

impl AppSettings {
    /// 解析 + 迁移。未知字段忽略（向前兼容）；老版本逐级升到当前。
    pub fn load(raw: &str) -> Result<Self, SettingsError> {
        if raw.trim().is_empty() {
            return Ok(Self::default());
        }
        let mut value: serde_json::Value =
            serde_json::from_str(raw).map_err(SettingsError::Parse)?;
        Self::migrate(&mut value)?;
        let mut settings: AppSettings =
            serde_json::from_value(value).map_err(SettingsError::Parse)?;
        // 兜底：增益越界 / 空语言等脏数据归一。
        settings.gain_db = settings.gain_db.clamp(-24.0, 24.0);
        settings.schema_version = SETTINGS_VERSION;
        Ok(settings)
    }

    fn migrate(value: &mut serde_json::Value) -> Result<(), SettingsError> {
        let version = value
            .get("schemaVersion")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32;

        if version < 2 {
            // v1 → v2：语言/主题枚举化；旧 build 只有 settings 中部分字段。
            if let Some(obj) = value.as_object_mut() {
                obj.entry("schemaVersion".to_string())
                    .or_insert_with(|| serde_json::json!(1));
                obj.entry("language".to_string())
                    .or_insert_with(|| serde_json::json!("system"));
                obj.entry("theme".to_string())
                    .or_insert_with(|| serde_json::json!("system"));
            }
        }
        // 当前即 v2。
        if let Some(obj) = value.as_object_mut() {
            obj.insert("schemaVersion".into(), serde_json::json!(SETTINGS_VERSION));
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("settings json parse failed: {0}")]
    Parse(#[from] serde_json::Error),
}

/// 语音会话历史的单条记录（本地存储，按日期分组）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceSessionRecord {
    pub started_at_ms: i64,
    pub duration_ms: u64,
    pub sample_count: u64,
    /// 会话结束时前台进程（小写短名）。
    pub foreground_process: Option<String>,
    pub profile_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_conservative() {
        let s = AppSettings::default();
        assert!(!s.onboarding_complete);
        assert_eq!(s.gain_db, 0.0);
        assert!(!s.profiles.smart_enabled);
        assert_eq!(s.schema_version, SETTINGS_VERSION);
    }

    #[test]
    fn roundtrip_serialization() {
        let s = AppSettings::default();
        let json = serde_json::to_string(&s).unwrap();
        let back = AppSettings::load(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn migrates_v1_settings() {
        let v1 = serde_json::json!({
            "schemaVersion": 1,
            "onboardingComplete": true,
            "gainDb": 6.0
        })
        .to_string();
        let s = AppSettings::load(&v1).unwrap();
        assert!(s.onboarding_complete);
        assert_eq!(s.gain_db, 6.0);
        assert_eq!(s.schema_version, SETTINGS_VERSION);
        // 迁移补默认。
        assert!(!s.launch_at_login);
    }

    #[test]
    fn empty_raw_falls_back_to_defaults() {
        assert_eq!(AppSettings::load("   ").unwrap(), AppSettings::default());
    }

    #[test]
    fn clamps_dirty_gain() {
        let raw = serde_json::json!({ "gainDb": 99.0 }).to_string();
        let s = AppSettings::load(&raw).unwrap();
        assert_eq!(s.gain_db, 24.0);
    }

    #[test]
    fn gain_applied_in_pipeline() {
        // 设置里的增益要真正作用于 PCM 管线。
        let s = AppSettings { gain_db: 6.0, ..Default::default() };
        let out = crate::pcm::postprocess(&[1000i16; 8], s.gain_db);
        assert!(out.iter().any(|&v| v > 1500));
    }
}
