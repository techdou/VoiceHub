//! 应用设置模型（版本化 + 迁移）。
//!
//! 存储由宿主负责（Tauri app_data_dir 下的 settings.json）；
//! 本模块只做结构、默认值与 v1 → 当前 的迁移。

use serde::{Deserialize, Serialize};

use crate::actions::ButtonAction;
use crate::profiles::ProfileStore;
use crate::provider::ProviderConfig;

pub const SETTINGS_VERSION: u32 = 2;

/// 录音键（语音键）的触发模式。
///
/// Ptt：按住录音键说话（固件蓝牙音频直传，默认行为）；
/// HandsFree：按一下开始、再按结束——蓝牙音频是固件"按住才推流"的语义，
/// 做不到 toggle，因此免提模式压掉蓝牙流，经系统麦克风录音。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VoiceKeyTriggerMode {
    #[default]
    Ptt,
    HandsFree,
}

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
    /// 录音键触发模式（免提 / 按住说话）。语音触发只由录音键承担，
    /// 其他键的旧触发绑定由 [`AppSettings::purge_legacy_voice_triggers`] 清除。
    pub voice_key_trigger_mode: VoiceKeyTriggerMode,
    pub launch_at_login: bool,
    pub language: Language,
    pub theme: Theme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum Language {
    #[default]
    System,
    ZhCn,
    English,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum Theme {
    #[default]
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
            voice_key_trigger_mode: VoiceKeyTriggerMode::Ptt,
            launch_at_login: false,
            language: Language::System,
            theme: Theme::System,
        }
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
        // 语音触发已收敛到录音键：读取即清除其他键上的旧触发绑定，
        // 保证按键行为只由 voice_key_trigger_mode 决定（迁移幂等）。
        settings.purge_legacy_voice_triggers();
        settings.normalize_single_profile();
        Ok(settings)
    }

    /// 按键映射页已收敛为单方案：关闭智能绑定并清空进程绑定表，让
    /// resolve_active 恒等于手动选中的方案——"配了就生效"不再依赖前台应用。
    /// 多套 profiles 数据保留（不破坏旧数据），仅不再提供切换与绑定入口。
    pub fn normalize_single_profile(&mut self) {
        self.profiles.smart_enabled = false;
        self.profiles.rules.process_bindings.clear();
        // smart 关闭后 resolve_active 恒等于选中方案，fallback 字段不影响行为，
        // 不重写（保持 roundtrip 稳定）。
        if self.profiles.selected_profile_id.is_empty() {
            self.profiles.selected_profile_id =
                self.profiles.profiles.first().map(|p| p.id.clone()).unwrap_or_default();
        }
    }

    /// 清除所有方案里其他键的语音触发绑定（pushToTalk / 免提动作）。
    /// 语音触发统一由录音键模式（voice_key_trigger_mode）承担。
    pub fn purge_legacy_voice_triggers(&mut self) {
        for profile in &mut self.profiles.profiles {
            for binding in profile.mapping.bindings.values_mut() {
                binding.push_to_talk = false;
                for slot in [&mut binding.single, &mut binding.double, &mut binding.long] {
                    if matches!(slot, ButtonAction::TriggerHandsFree) {
                        *slot = ButtonAction::Disabled;
                    }
                }
            }
        }
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

    #[test]
    fn voice_key_trigger_mode_defaults_to_ptt_for_legacy_settings() {
        // 旧 settings.json 没有该字段：读取必须落到 Ptt（按住说话），
        // 否则升级用户语音键行为静默翻转。
        let legacy = serde_json::json!({ "schemaVersion": 2, "onboardingComplete": true }).to_string();
        let s = AppSettings::load(&legacy).unwrap();
        assert_eq!(s.voice_key_trigger_mode, VoiceKeyTriggerMode::Ptt);
    }

    #[test]
    fn purge_clears_legacy_voice_triggers_and_keeps_other_actions() {
        use crate::mapping::{ButtonBinding, ButtonMapping};
        use ButtonAction::{Disabled, Shortcut, TriggerHandsFree};

        let shortcut = Shortcut { vk: 0x1b, modifiers: 0, label: "Esc".into() };
        let mut s = AppSettings::default();
        {
            let profile = s.profiles.selected_mut();
            profile.mapping = ButtonMapping {
                bindings: [
                    // ok：免提在 single 槽 + 正常动作在 double 槽 → 只清触发槽。
                    ("ok".into(), ButtonBinding {
                        single: TriggerHandsFree, double: shortcut.clone(), long: Disabled, push_to_talk: false,
                    }),
                    // back：按住说话第四通道 → 清开关。
                    ("back".into(), ButtonBinding {
                        single: Disabled, double: Disabled, long: Disabled, push_to_talk: true,
                    }),
                    // home：纯正常动作 → 原样保留。
                    ("home".into(), ButtonBinding::single(shortcut.clone())),
                ].into_iter().collect(),
            };
        }
        s.purge_legacy_voice_triggers();
        let bindings = &s.profiles.selected().mapping.bindings;
        assert_eq!(bindings["ok"].single, Disabled);
        assert_eq!(bindings["ok"].double, shortcut);
        assert!(!bindings["back"].push_to_talk);
        assert_eq!(bindings["home"].single, shortcut);
        // 幂等：再清一遍无变化。
        let again = s.clone();
        s.purge_legacy_voice_triggers();
        assert_eq!(s, again);
    }

    #[test]
    fn normalize_collapses_to_single_profile() {
        // 智能方案（按前台进程分方案）是"配了不生效"的一类根源：UI 已收敛为
        // 单方案，归一必须关掉 smart 绑定并清空进程表，选中方案保留。
        let mut s = AppSettings::default();
        s.profiles.smart_enabled = true;
        s.profiles.rules.process_bindings.insert("notepad.exe".into(), "p1".into());
        s.normalize_single_profile();
        assert!(!s.profiles.smart_enabled);
        assert!(s.profiles.rules.process_bindings.is_empty());
        assert_eq!(s.profiles.selected_profile_id, s.profiles.profiles[0].id);
    }

    #[test]
    fn load_purges_legacy_voice_triggers() {
        // 老版本把免提绑在 ok 键上：读取即清除，保证行为只由录音键模式决定。
        let mut raw = serde_json::to_value(AppSettings::default()).unwrap();
        raw["profiles"]["profiles"][0]["mapping"]["bindings"]["ok"] = serde_json::json!({
            "single": serde_json::to_value(ButtonAction::TriggerHandsFree).unwrap(),
            "double": { "kind": "disabled" },
            "long": { "kind": "disabled" },
            "pushToTalk": false
        });
        let s = AppSettings::load(&raw.to_string()).unwrap();
        assert_eq!(s.profiles.selected().mapping.bindings["ok"].single, ButtonAction::Disabled);
    }
}
