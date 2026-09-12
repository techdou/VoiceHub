//! 应用设置模型（版本化 + 迁移）。
//!
//! 存储由宿主负责（Tauri app_data_dir 下的 settings.json）；
//! 本模块只做结构、默认值与 v1 → 当前 的迁移。

use serde::{Deserialize, Serialize};

use crate::mapping::ButtonMapping;
use crate::provider::{legacy_shortcuts, ProviderConfig};

pub const SETTINGS_VERSION: u32 = 4;

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
    /// 按键映射（单方案）。多方案/按前台智能切换机制在 v4 拔除：
    /// UI 已收敛单方案许久，数据结构只是维护税。
    pub mapping: ButtonMapping,
    /// 按键自定义映射总开关（关闭 = 遥控器按键直通系统）。
    pub button_mapping_enabled: bool,
    /// 实验性：语音会话每 40s 发送 ATVV 续租（MIC_EXTEND），
    /// 尝试突破约 60s 固件会话边界。固件是否接受未真机验证。
    pub experimental_voice_extend: bool,
    /// 录音键触发模式（免提 / 按住说话）。语音触发只由录音键承担，
    /// 其他键的旧触发绑定由 [`AppSettings::purge_legacy_voice_triggers`] 清除。
    pub voice_key_trigger_mode: VoiceKeyTriggerMode,
    /// F5 拦截总开关：遥控器在线期间吞掉全部 F5（含真键盘），防止语音键
    /// 泄漏刷新前台页面。关闭后退回纯时序兜底（GATT 武装窗口 + 会话）。
    pub f5_gate_enabled: bool,
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
            mapping: crate::mapping::default_mapping(),
            button_mapping_enabled: true,
            experimental_voice_extend: false,
            voice_key_trigger_mode: VoiceKeyTriggerMode::Ptt,
            f5_gate_enabled: true,
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
        Ok(settings)
    }

    /// 清除其他键上的按住说话绑定（push_to_talk 开关）。语音触发统一由
    /// 录音键模式（voice_key_trigger_mode）承担；免提槽位动作已在 v4
    /// 迁移中随 TriggerHandsFree 枚举一并清洗为 Disabled。
    pub fn purge_legacy_voice_triggers(&mut self) {
        for binding in self.mapping.bindings.values_mut() {
            binding.push_to_talk = false;
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
        if version < 4 {
            // v3 → v4：拔除多方案机制——选中方案的映射提升为顶层 mapping，
            // profiles 数据整体丢弃（UI 已单方案许久，其余方案不可达）。
            if let Some(obj) = value.as_object_mut() {
                let existing_mapping = obj.get("mapping").cloned();
                let profiles_value = obj.remove("profiles");
                let selected_mapping = profiles_value.as_ref().and_then(|store| {
                    let list = store.get("profiles")?.as_array()?;
                    let selected_id = store.get("selectedProfileId").and_then(|v| v.as_str());
                    let chosen = list
                        .iter()
                        .find(|p| p.get("id").and_then(|v| v.as_str()) == selected_id)
                        .or_else(|| list.first());
                    chosen.and_then(|p| p.get("mapping")).cloned()
                });
                // 无 profiles 数据的异常配置落出厂默认映射，不让按键全空。
                // 优先级：profiles 选中方案 > 已有顶层 mapping（异常混载）> 出厂默认。
                let mut mapping = selected_mapping
                    .or(existing_mapping)
                    .unwrap_or_else(|| serde_json::to_value(crate::mapping::default_mapping()).unwrap());
                sanitize_removed_actions(&mut mapping);
                obj.insert("mapping".into(), mapping);
            }
        }
        if version < 3 {
            // v2 → v3：裁剪硬编码 Provider（微信输入法 / 豆包 / Win+H）。
            // 旧 kind 改写为 custom 并预填原默认键位——升级用户的外部工具
            // 触发行为不变；customVk 已非零（用户自己配过 Custom 键）则不覆盖。
            if let Some(provider) = value.get_mut("provider").and_then(|p| p.as_object_mut()) {
                let kind = provider
                    .get("kind")
                    .and_then(|k| k.as_str())
                    .unwrap_or_default()
                    .to_string();
                let legacy = match kind.as_str() {
                    "we_type" => Some((legacy_shortcuts::WETYPE_TOGGLE, "toggle")),
                    "doubao" => Some((legacy_shortcuts::DOUBAO_HOLD, "hold")),
                    "win_h" => Some((legacy_shortcuts::WIN_H, "toggle")),
                    _ => None,
                };
                if let Some(((vk, modifiers), mode)) = legacy {
                    let custom_unset = provider
                        .get("customVk")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0)
                        == 0;
                    provider.insert("kind".into(), serde_json::json!("custom"));
                    if custom_unset {
                        provider.insert("customVk".into(), serde_json::json!(vk));
                        provider.insert("customModifiers".into(), serde_json::json!(modifiers));
                        provider.insert("customMode".into(), serde_json::json!(mode));
                    }
                }
            }
        }
        // 当前即 v3。
        if let Some(obj) = value.as_object_mut() {
            obj.insert("schemaVersion".into(), serde_json::json!(SETTINGS_VERSION));
        }
        Ok(())
    }
}

/// v4 迁移的动作清洗：Custom → Shortcut（无损）、OpenUrl → OpenApp（等价，
/// 同走 shell::open_target）、其余被删动作 → Disabled。绑定值之外的未知
/// kind 不能留给反序列化失败——那会让整个 settings 回退默认、配置全丢。
fn sanitize_removed_actions(mapping: &mut serde_json::Value) {
    let Some(bindings) = mapping.get_mut("bindings").and_then(|b| b.as_object_mut()) else {
        return;
    };
    let rewrite = |slot: &mut serde_json::Value| {
        let kind = slot.get("kind").and_then(|k| k.as_str()).unwrap_or_default().to_string();
        match kind.as_str() {
            "custom" => {
                let inner = slot.get("shortcut").cloned().unwrap_or_default();
                if inner.get("vk").is_some() {
                    *slot = inner;
                    slot["kind"] = serde_json::json!("shortcut");
                } else {
                    *slot = serde_json::json!({ "kind": "disabled" });
                }
            }
            "open_url" => {
                let url = slot.get("url").cloned().unwrap_or_default();
                if url.is_string() && !url.as_str().unwrap_or_default().is_empty() {
                    *slot = serde_json::json!({
                        "kind": "open_app", "target": url, "label": url,
                    });
                } else {
                    *slot = serde_json::json!({ "kind": "disabled" });
                }
            }
            "screenshot" | "task_view" | "app_switcher" | "click_confirm"
            | "open_settings" | "trigger_hands_free" => {
                *slot = serde_json::json!({ "kind": "disabled" });
            }
            _ => {}
        }
    };
    for binding in bindings.values_mut() {
        for field in ["single", "double", "long"] {
            if let Some(slot) = binding.get_mut(field) {
                rewrite(slot);
            }
        }
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
        assert_eq!(s.mapping.bindings.len(), 8, "出厂默认映射应已预置 8 个键");
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
    fn f5_gate_defaults_on_for_legacy_settings() {
        // 旧 settings.json 没有该字段：读取必须落到 true（默认拦截），
        // 升级用户立即获得"遥控器在线吞 F5"保护。
        let legacy = serde_json::json!({ "schemaVersion": 2, "onboardingComplete": true }).to_string();
        let s = AppSettings::load(&legacy).unwrap();
        assert!(s.f5_gate_enabled);
    }

    #[test]
    fn migrates_legacy_provider_kinds_to_custom_with_default_shortcuts() {
        // v2 的硬编码 Provider 已裁剪：迁移必须改写为 custom 并预填原默认键位，
        // 否则 kind 反序列化失败 → 整个 settings 读取失败回退默认，用户配置全丢。
        for (wire, (vk, modifiers), mode) in [
            ("we_type", legacy_shortcuts::WETYPE_TOGGLE, "toggle"),
            ("doubao", legacy_shortcuts::DOUBAO_HOLD, "hold"),
            ("win_h", legacy_shortcuts::WIN_H, "toggle"),
        ] {
            let raw = serde_json::json!({
                "schemaVersion": 2,
                "provider": { "kind": wire }
            })
            .to_string();
            let s = AppSettings::load(&raw).unwrap();
            assert_eq!(s.provider.kind, crate::provider::ProviderKind::Custom, "{wire}");
            assert_eq!(s.provider.custom_vk, vk, "{wire}");
            assert_eq!(s.provider.custom_modifiers, modifiers, "{wire}");
            assert_eq!(
                s.provider.custom_mode,
                if mode == "toggle" {
                    crate::provider::TriggerMode::Toggle
                } else {
                    crate::provider::TriggerMode::Hold
                },
                "{wire}"
            );
        }
    }

    #[test]
    fn migration_keeps_user_configured_custom_shortcut() {
        // 用户曾配过 Custom 键又切回 we_type：customVk 非零，迁移只改 kind、
        // 不覆盖用户键位。
        let raw = serde_json::json!({
            "schemaVersion": 2,
            "provider": { "kind": "we_type", "customVk": 0x4B, "customModifiers": 2, "customMode": "hold" }
        })
        .to_string();
        let s = AppSettings::load(&raw).unwrap();
        assert_eq!(s.provider.kind, crate::provider::ProviderKind::Custom);
        assert_eq!(s.provider.custom_vk, 0x4B);
        assert_eq!(s.provider.custom_modifiers, 2);
        assert_eq!(s.provider.custom_mode, crate::provider::TriggerMode::Hold);
    }

    #[test]
    fn purge_clears_legacy_voice_triggers_and_keeps_other_actions() {
        use crate::mapping::{ButtonBinding, ButtonMapping};
        use crate::actions::ButtonAction::{Disabled, Shortcut};

        let shortcut = Shortcut { vk: 0x1b, modifiers: 0, label: "Esc".into() };
        let mut s = AppSettings::default();
        s.mapping = ButtonMapping {
            bindings: [
                // ok：免提在 single 槽 + 正常动作在 double 槽 → 只清触发槽。
                ("ok".into(), ButtonBinding {
                    single: shortcut.clone(), double: Disabled, long: Disabled, push_to_talk: false,
                }),
                // back：按住说话第四通道 → 清开关。
                ("back".into(), ButtonBinding {
                    single: Disabled, double: Disabled, long: Disabled, push_to_talk: true,
                }),
                // home：纯正常动作 → 原样保留。
                ("home".into(), ButtonBinding::single(shortcut.clone())),
            ].into_iter().collect(),
        };
        s.purge_legacy_voice_triggers();
        let bindings = &s.mapping.bindings;
        // 正常动作不受清洗影响；被按住说话占用的键只清开关。
        assert_eq!(bindings["ok"].single, shortcut);
        assert_eq!(bindings["ok"].double, Disabled);
        assert!(!bindings["back"].push_to_talk);
        assert_eq!(bindings["home"].single, shortcut);
        // 幂等：再清一遍无变化。
        let again = s.clone();
        s.purge_legacy_voice_triggers();
        assert_eq!(s, again);
    }

    #[test]
    fn migrates_v3_profiles_to_top_level_mapping() {
        // v3 多方案已拔除：选中方案的 mapping 必须无损提升，其余方案丢弃。
        let v3 = serde_json::json!({
            "schemaVersion": 3,
            "profiles": {
                "profiles": [
                    { "id": "p1", "name": "通用", "icon": "",
                      "mapping": { "bindings": { "up": { "single": { "kind": "volume_up" }, "double": { "kind": "disabled" }, "long": { "kind": "disabled" }, "pushToTalk": false } } } },
                    { "id": "p2", "name": "第二套", "icon": "",
                      "mapping": { "bindings": { "ok": { "single": { "kind": "show_desktop" }, "double": { "kind": "disabled" }, "long": { "kind": "disabled" }, "pushToTalk": false } } } }
                ],
                "selectedProfileId": "p2",
                "smartEnabled": false,
                "rules": { "processBindings": {}, "fallbackProfileId": "" }
            }
        })
        .to_string();
        let s = AppSettings::load(&v3).unwrap();
        assert!(s.mapping.bindings.contains_key("ok"), "选中方案 p2 的绑定应提升为顶层");
        assert!(!s.mapping.bindings.contains_key("up"), "未选中方案 p1 的绑定应丢弃");
        assert_eq!(s.schema_version, SETTINGS_VERSION);
    }

    #[test]
    fn migrates_removed_actions_losslessly_or_to_disabled() {
        let v3 = serde_json::json!({
            "schemaVersion": 3,
            "mapping": { "bindings": {
                "up": { "single": { "kind": "custom", "shortcut": { "vk": 0x56, "modifiers": 2, "label": "Ctrl+V" } }, "double": { "kind": "open_url", "url": "https://example.com" }, "long": { "kind": "disabled" }, "pushToTalk": false },
                "ok": { "single": { "kind": "task_view" }, "double": { "kind": "screenshot", "region": true }, "long": { "kind": "trigger_hands_free" }, "pushToTalk": false }
            } }
        })
        .to_string();
        let s = AppSettings::load(&v3).unwrap();
        let up = &s.mapping.bindings["up"];
        assert_eq!(up.single, crate::actions::ButtonAction::Shortcut { vk: 0x56, modifiers: 2, label: "Ctrl+V".into() });
        assert_eq!(up.double, crate::actions::ButtonAction::OpenApp { target: "https://example.com".into(), label: "https://example.com".into() });
        let ok = &s.mapping.bindings["ok"];
        assert_eq!(ok.single, crate::actions::ButtonAction::Disabled);
        assert_eq!(ok.double, crate::actions::ButtonAction::Disabled);
        assert_eq!(ok.long, crate::actions::ButtonAction::Disabled);
    }

    #[test]
    fn migrates_v3_without_profiles_to_default_mapping() {
        let v3 = serde_json::json!({ "schemaVersion": 3 }).to_string();
        let s = AppSettings::load(&v3).unwrap();
        assert_eq!(s.mapping, crate::mapping::default_mapping());
    }

    #[test]
    fn load_purges_legacy_voice_triggers() {
        // 老版本把免提绑在 ok 键上：读取即清除，保证行为只由录音键模式决定。
        let mut raw = serde_json::to_value(AppSettings::default()).unwrap();
        raw["mapping"]["bindings"]["ok"] = serde_json::json!({
            "single": { "kind": "volume_up" },
            "double": { "kind": "disabled" },
            "long": { "kind": "disabled" },
            "pushToTalk": true
        });
        let s = AppSettings::load(&raw.to_string()).unwrap();
        assert!(!s.mapping.bindings["ok"].push_to_talk, "旧版按住说话绑定读取即清除");
    }
}
