//! Profiles：多套键位方案 + 按前台应用自动匹配（Smart Profiles）。

use serde::{Deserialize, Serialize};

use crate::actions::ButtonAction;
use crate::mapping::ButtonMapping;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ButtonProfile {
    pub id: String,
    pub name: String,
    /// 图标标识（UI 内置图标名或首字母）。
    pub icon: String,
    pub mapping: ButtonMapping,
}

static PROFILE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn new_profile_id() -> String {
    let seq = PROFILE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let millis = chrono::Utc::now().timestamp_millis().max(0) as u64;
    format!("profile-{millis:x}-{seq:x}")
}

impl ButtonProfile {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: new_profile_id(),
            name: name.into(),
            icon: String::new(),
            mapping: ButtonMapping::default(),
        }
    }
}

/// Smart Profiles：进程名（小写、不带 .exe）→ Profile。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ProfileRules {
    /// 前台进程 → Profile ID。
    pub process_bindings: std::collections::HashMap<String, String>,
    /// 无匹配时的回退 Profile ID；空 → 用 selected_profile_id。
    pub fallback_profile_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileStore {
    pub profiles: Vec<ButtonProfile>,
    pub selected_profile_id: String,
    /// Smart Profiles 默认关闭（参考实现的保守默认）。
    pub smart_enabled: bool,
    pub rules: ProfileRules,
}

impl Default for ProfileStore {
    fn default() -> Self {
        Self::with_preset_profiles()
    }
}

impl ProfileStore {
    pub fn with_preset_profiles() -> Self {
        let general = ButtonProfile {
            id: "preset-general".into(),
            name: "通用导航".into(),
            icon: "compass".into(),
            mapping: crate::mapping::default_mapping(),
        };
        let mut coding = ButtonProfile {
            id: "preset-coding".into(),
            name: "Vibe Coding".into(),
            icon: "spark".into(),
            mapping: ButtonMapping::default(),
        };
        {
            use crate::actions::vk;
            use crate::actions::MOD_SHIFT;
            use crate::mapping::ButtonBinding;
            let enter = |m: u8, label: &str| {
                ButtonBinding::single(ButtonAction::Shortcut {
                    vk: vk::RETURN,
                    modifiers: m,
                    label: label.into(),
                })
            };
            coding.mapping.set(
                RemoteButtonForProfile::Ok,
                enter(0, "Enter"),
            );
            let mut home = ButtonBinding::default();
            home.single = ButtonAction::TaskView;
            coding.mapping.set(RemoteButtonForProfile::Home, home);
            let mut tv = ButtonBinding::default();
            tv.single = ButtonAction::Shortcut {
                vk: vk::L,
                modifiers: MOD_SHIFT,
                label: "Shift+L".into(),
            };
            coding.mapping.set(RemoteButtonForProfile::Tv, tv);
        }
        Self {
            profiles: vec![general, coding],
            selected_profile_id: "preset-general".into(),
            smart_enabled: false,
            rules: ProfileRules::default(),
        }
    }

    pub fn selected(&self) -> &ButtonProfile {
        self.profiles
            .iter()
            .find(|p| p.id == self.selected_profile_id)
            .unwrap_or(&self.profiles[0])
    }

    pub fn selected_mut(&mut self) -> &mut ButtonProfile {
        let selected_id = self.selected().id.clone();
        self.profiles
            .iter_mut()
            .find(|p| p.id == selected_id)
            .expect("selected profile must exist")
    }

    /// 按前台进程解析生效 Profile。
    ///
    /// 关闭 Smart → 恒为手动选择的方案；开启 → 精确匹配进程名，
    /// 否则 fallback（未配置则回退手动选择）。
    pub fn resolve_active(&self, foreground_process: Option<&str>) -> &ButtonProfile {
        if !self.smart_enabled {
            return self.selected();
        }
        let Some(process) = foreground_process else {
            return self.fallback_profile();
        };
        let normalized = normalize_process_name(process);
        if let Some(profile_id) = self.rules.process_bindings.get(&normalized) {
            if let Some(profile) = self.profiles.iter().find(|p| p.id == *profile_id) {
                return profile;
            }
        }
        self.fallback_profile()
    }

    fn fallback_profile(&self) -> &ButtonProfile {
        self.profiles
            .iter()
            .find(|p| p.id == self.rules.fallback_profile_id)
            .unwrap_or_else(|| self.selected())
    }

    /// 删除方案：不允许删到空；删除被选中的方案时回退到第一个。
    pub fn remove(&mut self, id: &str) -> Result<(), ProfileError> {
        if self.profiles.len() <= 1 {
            return Err(ProfileError::CannotRemoveLastProfile);
        }
        let before = self.profiles.len();
        self.profiles.retain(|p| p.id != id);
        if self.profiles.len() == before {
            return Err(ProfileError::ProfileNotFound(id.into()));
        }
        self.rules.process_bindings.retain(|_, v| v != id);
        if self.rules.fallback_profile_id == id {
            self.rules.fallback_profile_id = String::new();
        }
        if self.selected_profile_id == id {
            self.selected_profile_id = self.profiles[0].id.clone();
        }
        Ok(())
    }

    /// 进程绑定校验：一个进程只能绑一个方案，一个方案可被多个进程绑定。
    pub fn bind_process(&mut self, process: &str, profile_id: &str) -> Result<(), ProfileError> {
        if !self.profiles.iter().any(|p| p.id == profile_id) {
            return Err(ProfileError::ProfileNotFound(profile_id.into()));
        }
        self.rules
            .process_bindings
            .insert(normalize_process_name(process), profile_id.to_string());
        Ok(())
    }
}

/// normalize：去路径、去 .exe 后缀、小写（大小写不敏感处理）。
pub fn normalize_process_name(process: &str) -> String {
    let lowered = process.to_ascii_lowercase();
    let name = lowered.rsplit(['\\', '/']).next().unwrap_or(&lowered);
    name.strip_suffix(".exe").unwrap_or(name).to_string()
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProfileError {
    #[error("cannot remove the last profile")]
    CannotRemoveLastProfile,
    #[error("profile not found: {0}")]
    ProfileNotFound(String),
}

// 按钮别名供上方 preset 构造使用。
use crate::buttons::RemoteButton as RemoteButtonForProfile;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buttons::RemoteButton;
    use crate::gesture::Gesture;

    fn store_with_binding() -> ProfileStore {
        let mut store = ProfileStore::default();
        store.smart_enabled = true;
        let coding_id = store.profiles.iter().find(|p| p.name == "Vibe Coding").unwrap().id.clone();
        store.bind_process("Cursor.exe", &coding_id).unwrap();
        store
    }

    #[test]
    fn smart_disabled_ignores_foreground() {
        let mut store = ProfileStore::default();
        store.smart_enabled = false;
        let second = store.profiles[1].id.clone();
        store.bind_process("cursor", &second).unwrap();
        let active = store.resolve_active(Some("Cursor.exe"));
        assert_eq!(active.id, "preset-general");
    }

    #[test]
    fn smart_matches_normalized_process() {
        let store = store_with_binding();
        let active = store.resolve_active(Some("C:\\Apps\\CURSOR.EXE"));
        assert_eq!(active.name, "Vibe Coding");
    }

    #[test]
    fn smart_falls_back_when_unmatched() {
        let store = store_with_binding();
        let active = store.resolve_active(Some("notepad.exe"));
        assert_eq!(active.id, "preset-general");
    }

    #[test]
    fn smart_fallback_profile_used_when_configured() {
        let mut store = ProfileStore::default();
        store.smart_enabled = true;
        let second = store.profiles[1].id.clone();
        store.rules.fallback_profile_id = second.clone();
        let active = store.resolve_active(Some("notepad.exe"));
        assert_eq!(active.id, second);
    }

    #[test]
    fn missing_foreground_uses_fallback() {
        let store = store_with_binding();
        assert_eq!(store.resolve_active(None).id, "preset-general");
    }

    #[test]
    fn cannot_remove_last_profile() {
        let mut store = ProfileStore::default();
        let first = store.profiles[1].id.clone();
        store.remove(&first).unwrap();
        let last_id = store.profiles[0].id.clone();
        assert_eq!(
            store.remove(&last_id),
            Err(ProfileError::CannotRemoveLastProfile)
        );
    }

    #[test]
    fn removing_selected_falls_back_to_first() {
        let mut store = ProfileStore::default();
        let general = store.profiles[0].id.clone();
        store.selected_profile_id = general.clone();
        store.remove(&general).unwrap();
        assert_eq!(store.selected_profile_id, store.profiles[0].id);
    }

    #[test]
    fn preset_profiles_resolve_actions() {
        let store = ProfileStore::default();
        let active = store.resolve_active(Some("whatever.exe"));
        assert!(active.mapping.resolve(RemoteButton::Up, Gesture::SingleClick).is_some());
    }

    #[test]
    fn bind_rejects_unknown_profile() {
        let mut store = ProfileStore::default();
        assert_eq!(
            store.bind_process("a.exe", "no-such-id"),
            Err(ProfileError::ProfileNotFound("no-such-id".into()))
        );
    }
}
