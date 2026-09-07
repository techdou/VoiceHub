// 备份 / 恢复：配置（JSON）与全部数据（zip，含音频）的导出与导入。
//
// 设计要点：
// - 导入走活动数据库的正常写入通道（app_settings upsert + 集合表整表替换），
//   不替换 sqlite 文件、不涉及文件锁，导入完成后由前端触发重启使内存状态重载。
// - 语义为「覆盖」：集合表整表替换；app_settings 逐 key 覆盖（不删除未出现的 key）。
// - 「配置」不含使用统计 stats，避免覆盖本机计数；「全部」包含 stats。
// - 全部导入时，历史记录里的 audioFilePath 是旧机绝对路径，会重写为本机 audio 目录下的同名文件。

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

use crate::storage::Storage;

/// 备份文件格式版本。导入时若备份版本高于此值则拒绝。
const FORMAT_VERSION: i64 = 1;
/// 分项配置文件格式；完整配置与全部数据继续使用 v1，保持兼容。
const SELECTED_CONFIG_FORMAT_VERSION: i64 = 2;
const MAX_HOTWORDS: usize = 1000;
const BUILTIN_PRESET_IDS: &[&str] = &["intent", "faithful", "zh2en", "casual"];
/// 「配置」档不导出/导入的 app_settings key（使用统计属于使用数据，不属于配置）。
const CONFIG_EXCLUDE: &[&str] = &["stats"];

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigExportSelection {
    mode: String,
    #[serde(default)]
    hotword_group_ids: Vec<String>,
    #[serde(default)]
    include_text_replacements: bool,
    #[serde(default)]
    text_replacements: Option<Vec<Value>>,
    #[serde(default)]
    prompt_preset_ids: Vec<String>,
}

/// 全量备份里除配置之外还装什么。
///
/// 两个字段都**不给 serde 默认值**：调用方必须显式表态。给默认值的那一版是个陷阱 ——
/// 漏传参数时会静默按「全都要」打包，而含音频的包可以有几个 GB，自动备份场景下
/// 意味着每次悄悄上传几个 GB。宁可让反序列化直接失败，报一条看得见的错。
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupScope {
    pub include_history: bool,
    pub include_audio: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigImportSectionPreview {
    kind: String,
    total: usize,
    added: usize,
    updated: usize,
    skipped: usize,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigImportWarning {
    code: String,
    current: Option<usize>,
    limit: Option<usize>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigImportPreview {
    scope: String,
    format_version: i64,
    import_token: String,
    sections: Vec<ConfigImportSectionPreview>,
    warnings: Vec<ConfigImportWarning>,
    requires_restart: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigImportResult {
    changed_sections: Vec<String>,
    added: usize,
    updated: usize,
    skipped: usize,
    requires_restart: bool,
}

struct SelectedImportPlan {
    app_settings: Map<String, Value>,
    prompt_presets: Option<Vec<Value>>,
    sections: Vec<ConfigImportSectionPreview>,
    warnings: Vec<ConfigImportWarning>,
    added: usize,
    updated: usize,
    skipped: usize,
}

fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn audio_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("app.soundbridge.windows/sayit")
        .join("audio")
}

fn backup_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(dirs::document_dir)
        .unwrap_or_else(|| {
            dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("app.soundbridge.windows/sayit")
        })
        .join("SayIt Backups")
}

fn timestamped_backup_path(prefix: &str, extension: &str) -> PathBuf {
    let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S-%3f");
    backup_dir().join(format!("{}-{}.{}", prefix, timestamp, extension))
}

/// 取路径的文件名部分（兼容 / 和 \ 分隔符，跨平台备份用）。
fn basename(path: &str) -> String {
    path.rsplit(|c| c == '/' || c == '\\')
        .next()
        .unwrap_or(path)
        .to_string()
}

fn build_config_value(storage: &Storage) -> Value {
    json!({
        "kind": "config",
        "formatVersion": FORMAT_VERSION,
        "appVersion": app_version(),
        "exportedAt": chrono::Utc::now().to_rfc3339(),
        "appSettings": storage.export_app_settings(CONFIG_EXCLUDE),
        "promptPresets": storage.get("promptPresets", None),
        "appPromptRules": storage.get("appPromptRules", None),
    })
}

fn normalized_strings(value: Option<&Value>) -> Vec<String> {
    let mut seen = HashSet::new();
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .filter(|item| seen.insert((*item).to_string()))
        .map(ToString::to_string)
        .collect()
}

fn build_selected_config_value(
    storage: &Storage,
    selection: &ConfigExportSelection,
) -> Result<Value, String> {
    let selected_hotword_ids: HashSet<&str> = selection
        .hotword_group_ids
        .iter()
        .map(String::as_str)
        .collect();
    let selected_preset_ids: HashSet<&str> = selection
        .prompt_preset_ids
        .iter()
        .map(String::as_str)
        .collect();
    let mut items = Map::new();

    if !selected_hotword_ids.is_empty() {
        let themes_value = storage.get("customHotwordThemes", None);
        let active_value = storage.get("customThemeActive", None);
        let active_map = active_value.as_object();
        let groups = themes_value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_object)
            .filter(|theme| {
                theme
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| selected_hotword_ids.contains(id))
            })
            .filter_map(|theme| {
                let id = theme.get("id")?.as_str()?;
                let name = theme.get("name")?.as_str()?.trim();
                if name.is_empty() {
                    return None;
                }
                Some(json!({
                    "name": name,
                    "words": normalized_strings(theme.get("words")),
                    "enabled": active_map
                        .and_then(|map| map.get(id))
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                }))
            })
            .collect::<Vec<_>>();
        items.insert("hotwordGroups".to_string(), Value::Array(groups));
    }

    if selection.include_text_replacements {
        let source_rules = selection.text_replacements.clone().unwrap_or_else(|| {
            storage
                .get("textReplacements", None)
                .as_array()
                .cloned()
                .unwrap_or_default()
        });
        let rules = source_rules
            .into_iter()
            .filter_map(|rule| {
                let obj = rule.as_object()?;
                let from = obj.get("from")?.as_str()?.trim();
                if from.is_empty() {
                    return None;
                }
                Some(json!({
                    "from": from,
                    "to": obj.get("to").and_then(Value::as_str).unwrap_or(""),
                    "enabled": obj.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                }))
            })
            .collect::<Vec<_>>();
        items.insert("textReplacements".to_string(), Value::Array(rules));
    }

    if !selected_preset_ids.is_empty() {
        let presets = storage
            .get("promptPresets", None)
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|preset| {
                let obj = preset.as_object()?;
                let id = obj.get("id")?.as_str()?;
                if !selected_preset_ids.contains(id) || BUILTIN_PRESET_IDS.contains(&id) {
                    return None;
                }
                let name = obj.get("name")?.as_str()?.trim();
                let system_prompt = obj.get("systemPrompt")?.as_str()?.trim();
                if name.is_empty() || system_prompt.is_empty() {
                    return None;
                }
                Some(json!({ "name": name, "systemPrompt": system_prompt }))
            })
            .collect::<Vec<_>>();
        items.insert("promptPresets".to_string(), Value::Array(presets));
    }

    if items.is_empty() {
        return Err("Select at least one configuration item to export".to_string());
    }

    Ok(json!({
        "kind": "config",
        "formatVersion": SELECTED_CONFIG_FORMAT_VERSION,
        "scope": "selected",
        "appVersion": app_version(),
        "exportedAt": chrono::Utc::now().to_rfc3339(),
        "items": Value::Object(items),
    }))
}

/// 组装 `backup.json`。
///
/// 不含历史时**整个字段都不写**（而不是写一个空数组）：`import_full` 对缺失段是
/// `if let Some(...)`，字段缺席就跳过、保留本机现有历史；写成空数组反而会把本机
/// 历史整表清空。「只备份配置」这一档能安全恢复，全靠这个区别。
fn build_full_value(storage: &Storage, scope: BackupScope) -> Value {
    let mut payload = json!({
        "kind": "full",
        "formatVersion": FORMAT_VERSION,
        "appVersion": app_version(),
        "exportedAt": chrono::Utc::now().to_rfc3339(),
        // 备份包自报装了什么，供恢复前预览与排查「为什么恢复完没有历史」。
        "scope": {
            "includeHistory": scope.include_history,
            "includeAudio": scope.include_audio,
        },
        "appSettings": storage.export_app_settings(&[]),
        "promptPresets": storage.get("promptPresets", None),
        "appPromptRules": storage.get("appPromptRules", None),
    });

    if scope.include_history {
        if let Some(object) = payload.as_object_mut() {
            object.insert("history".to_string(), storage.get("history", None));
            object.insert(
                "manualCorrections".to_string(),
                storage.get("manualCorrections", None),
            );
            object.insert(
                "feedbackQueue".to_string(),
                storage.get("feedbackQueue", None),
            );
        }
    }

    payload
}

fn check_kind_and_version(data: &Value, expected_kind: &str, max_version: i64) -> Result<i64, String> {
    let kind = data.get("kind").and_then(Value::as_str).unwrap_or("");
    if kind != expected_kind {
        return Err(format!("This is not a valid SayIt {} file", if expected_kind == "full" { "full-data backup" } else { "configuration" }));
    }
    let version = data
        .get("formatVersion")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if version > max_version {
        return Err(format!(
            "The backup version ({}) is newer than this app supports ({}). Update SayIt before importing it.",
            version, max_version
        ));
    }
    Ok(version)
}

fn is_selected_config(data: &Value) -> bool {
    data.get("scope").and_then(Value::as_str) == Some("selected")
        && data.get("formatVersion").and_then(Value::as_i64)
            == Some(SELECTED_CONFIG_FORMAT_VERSION)
}

/// 应用完整配置部分（设置 + Prompt 预设 + 分应用规则），两档共用。
fn apply_config_part(
    storage: &Storage,
    data: &Value,
    settings_exclude: &[&str],
) -> Result<(), String> {
    let empty_settings = Map::new();
    let app_settings = data
        .get("appSettings")
        .and_then(Value::as_object)
        .unwrap_or(&empty_settings);
    let prompt_presets = data
        .get("promptPresets")
        .and_then(Value::as_array)
        .map(Vec::as_slice);
    let app_prompt_rules = data
        .get("appPromptRules")
        .and_then(Value::as_array)
        .map(Vec::as_slice);

    storage
        .apply_config_transaction(
            app_settings,
            settings_exclude,
            prompt_presets,
            app_prompt_rules,
        )
        .map_err(|error| format!("Failed to write configuration: {}", error))
}

/// 重写单条历史记录的音频路径：把目录段换成本机 audio 目录，文件名不变。
fn rewrite_audio_path(rec: &Value, adir: &Path) -> Value {
    let mut rec = rec.clone();
    if let Some(obj) = rec.as_object_mut() {
        if let Some(p) = obj.get("audioFilePath").and_then(|v| v.as_str()).map(|s| s.to_string()) {
            let base = basename(&p);
            if !base.is_empty() {
                let new_path = adir.join(&base).to_string_lossy().to_string();
                obj.insert("audioFilePath".to_string(), Value::String(new_path));
            }
        }
    }
    rec
}

fn next_import_id(prefix: &str, used_ids: &mut HashSet<String>, sequence: &mut usize) -> String {
    loop {
        let candidate = format!(
            "{}_{}_{}",
            prefix,
            chrono::Utc::now().timestamp_millis(),
            *sequence
        );
        *sequence += 1;
        if used_ids.insert(candidate.clone()) {
            return candidate;
        }
    }
}

fn unique_import_name(base: &str, used_names: &mut HashSet<String>) -> String {
    if used_names.insert(base.to_lowercase()) {
        return base.to_string();
    }
    let mut index = 1usize;
    loop {
        let candidate = if index == 1 {
            format!("{} (Imported)", base)
        } else {
            format!("{} (Imported {})", base, index)
        };
        if used_names.insert(candidate.to_lowercase()) {
            return candidate;
        }
        index += 1;
    }
}

fn build_selected_import_plan(storage: &Storage, data: &Value) -> Result<SelectedImportPlan, String> {
    let items = data
        .get("items")
        .and_then(Value::as_object)
        .ok_or_else(|| "The configuration file is missing items and may be damaged".to_string())?;
    let mut app_settings = Map::new();
    let mut prompt_presets = None;
    let mut sections = Vec::new();
    let mut warnings = Vec::new();
    let mut total_added = 0usize;
    let mut total_updated = 0usize;
    let mut total_skipped = 0usize;
    let mut sequence = 0usize;

    if let Some(group_value) = items.get("hotwordGroups") {
        let imported_groups = group_value
            .as_array()
            .ok_or_else(|| "Invalid hotword group format".to_string())?;
        let mut local_groups = storage
            .get("customHotwordThemes", None)
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut active_map = storage
            .get("customThemeActive", None)
            .as_object()
            .cloned()
            .unwrap_or_default();
        let mut used_ids = local_groups
            .iter()
            .filter_map(|group| group.get("id").and_then(Value::as_str))
            .map(ToString::to_string)
            .collect::<HashSet<_>>();
        let mut added = 0usize;
        let mut updated = 0usize;
        let mut skipped = 0usize;

        for imported in imported_groups {
            let Some(obj) = imported.as_object() else {
                skipped += 1;
                continue;
            };
            let name = obj
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("");
            if name.is_empty() {
                skipped += 1;
                continue;
            }
            let words = normalized_strings(obj.get("words"));
            let normalized_name = name.to_lowercase();
            let existing_index = local_groups.iter().position(|group| {
                group
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|current| current.trim().to_lowercase() == normalized_name)
            });

            if let Some(index) = existing_index {
                let existing_words = normalized_strings(local_groups[index].get("words"));
                let mut merged_words = existing_words.clone();
                let mut seen = existing_words.into_iter().collect::<HashSet<_>>();
                for word in words {
                    if seen.insert(word.clone()) {
                        merged_words.push(word);
                    }
                }
                if merged_words.len()
                    == normalized_strings(local_groups[index].get("words")).len()
                {
                    skipped += 1;
                } else if let Some(local_obj) = local_groups[index].as_object_mut() {
                    local_obj.insert("words".to_string(), json!(merged_words));
                    updated += 1;
                }
            } else {
                let id = next_import_id("theme_import", &mut used_ids, &mut sequence);
                let enabled = obj
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                local_groups.push(json!({ "id": id, "name": name, "words": words }));
                active_map.insert(id, Value::Bool(enabled));
                added += 1;
            }
        }

        let mut active_words = HashSet::new();
        for group in &local_groups {
            let Some(id) = group.get("id").and_then(Value::as_str) else {
                continue;
            };
            if active_map.get(id).and_then(Value::as_bool) != Some(true) {
                continue;
            }
            active_words.extend(normalized_strings(group.get("words")));
        }
        if active_words.len() > MAX_HOTWORDS {
            warnings.push(ConfigImportWarning {
                code: "hotwordLimit".to_string(),
                current: Some(active_words.len()),
                limit: Some(MAX_HOTWORDS),
            });
        }

        app_settings.insert("customHotwordThemes".to_string(), Value::Array(local_groups));
        app_settings.insert("customThemeActive".to_string(), Value::Object(active_map));
        sections.push(ConfigImportSectionPreview {
            kind: "hotwords".to_string(),
            total: imported_groups.len(),
            added,
            updated,
            skipped,
        });
        total_added += added;
        total_updated += updated;
        total_skipped += skipped;
    }

    if let Some(rules_value) = items.get("textReplacements") {
        let imported_rules = rules_value
            .as_array()
            .ok_or_else(|| "Invalid text replacement format".to_string())?;
        let mut local_rules = storage
            .get("textReplacements", None)
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut used_ids = local_rules
            .iter()
            .filter_map(|rule| rule.get("id").and_then(Value::as_str))
            .map(ToString::to_string)
            .collect::<HashSet<_>>();
        let mut added = 0usize;
        let mut updated = 0usize;
        let mut skipped = 0usize;

        for imported in imported_rules {
            let Some(obj) = imported.as_object() else {
                skipped += 1;
                continue;
            };
            let from = obj
                .get("from")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("");
            if from.is_empty() {
                skipped += 1;
                continue;
            }
            let to = obj.get("to").and_then(Value::as_str).unwrap_or("");
            let existing_index = local_rules.iter().position(|rule| {
                rule.get("from").and_then(Value::as_str) == Some(from)
            });

            if let Some(index) = existing_index {
                let current_to = local_rules[index]
                    .get("to")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if current_to == to {
                    skipped += 1;
                } else if let Some(local_obj) = local_rules[index].as_object_mut() {
                    local_obj.insert("to".to_string(), Value::String(to.to_string()));
                    updated += 1;
                }
            } else {
                let id = next_import_id("replacement_import", &mut used_ids, &mut sequence);
                local_rules.push(json!({
                    "id": id,
                    "from": from,
                    "to": to,
                    "enabled": obj.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                }));
                added += 1;
            }
        }

        app_settings.insert("textReplacements".to_string(), Value::Array(local_rules));
        sections.push(ConfigImportSectionPreview {
            kind: "textReplacements".to_string(),
            total: imported_rules.len(),
            added,
            updated,
            skipped,
        });
        total_added += added;
        total_updated += updated;
        total_skipped += skipped;
    }

    if let Some(presets_value) = items.get("promptPresets") {
        let imported_presets = presets_value
            .as_array()
            .ok_or_else(|| "Invalid prompt preset format".to_string())?;
        let mut local_presets = storage
            .get("promptPresets", None)
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut used_ids = local_presets
            .iter()
            .filter_map(|preset| preset.get("id").and_then(Value::as_str))
            .map(ToString::to_string)
            .collect::<HashSet<_>>();
        let mut used_names = local_presets
            .iter()
            .filter_map(|preset| preset.get("name").and_then(Value::as_str))
            .map(|name| name.trim().to_lowercase())
            .collect::<HashSet<_>>();
        let mut added = 0usize;
        let mut skipped = 0usize;

        for imported in imported_presets {
            let Some(obj) = imported.as_object() else {
                skipped += 1;
                continue;
            };
            let name = obj
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("");
            let system_prompt = obj
                .get("systemPrompt")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("");
            if name.is_empty() || system_prompt.is_empty() {
                skipped += 1;
                continue;
            }
            let exact_duplicate = local_presets.iter().any(|preset| {
                preset
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|current| current.trim().eq_ignore_ascii_case(name))
                    && preset.get("systemPrompt").and_then(Value::as_str) == Some(system_prompt)
            });
            if exact_duplicate {
                skipped += 1;
                continue;
            }
            let imported_name = unique_import_name(name, &mut used_names);
            let id = next_import_id("preset_import", &mut used_ids, &mut sequence);
            local_presets.push(json!({
                "id": id,
                "name": imported_name,
                "systemPrompt": system_prompt,
            }));
            added += 1;
        }

        prompt_presets = Some(local_presets);
        sections.push(ConfigImportSectionPreview {
            kind: "promptPresets".to_string(),
            total: imported_presets.len(),
            added,
            updated: 0,
            skipped,
        });
        total_added += added;
        total_skipped += skipped;
    }

    if sections.is_empty() {
        return Err("The configuration file contains nothing that can be imported".to_string());
    }

    Ok(SelectedImportPlan {
        app_settings,
        prompt_presets,
        sections,
        warnings,
        added: total_added,
        updated: total_updated,
        skipped: total_skipped,
    })
}

// ─── 导出 ───

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupExportProgress {
    status: String,
    phase: String,
    file_path: String,
    current_file: Option<String>,
    processed_files: u64,
    total_files: u64,
    processed_bytes: u64,
    total_bytes: u64,
    percent: f64,
    error: Option<String>,
}

struct AudioExportFile {
    path: PathBuf,
    name: String,
    size: u64,
}

fn emit_export_progress(app: &AppHandle, progress: BackupExportProgress) {
    let _ = app.emit("backup-export-progress", progress);
}

fn collect_audio_export_files(include_audio: bool) -> Vec<AudioExportFile> {
    let mut files = Vec::new();
    if !include_audio {
        return files;
    }
    let adir = audio_dir();
    if let Ok(entries) = fs::read_dir(adir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.is_empty() {
                files.push(AudioExportFile {
                    path,
                    name,
                    size: metadata.len(),
                });
            }
        }
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    files
}

fn audio_progress_percent(processed_bytes: u64, total_bytes: u64) -> f64 {
    if total_bytes == 0 {
        95.0
    } else {
        (5.0 + processed_bytes as f64 / total_bytes as f64 * 90.0).min(95.0)
    }
}

#[tauri::command]
pub fn get_backup_directory() -> Result<String, String> {
    let directory = backup_dir();
    fs::create_dir_all(&directory).map_err(|e| format!("Failed to create backup directory: {}", e))?;
    Ok(directory.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn export_config(
    selection: ConfigExportSelection,
    storage: State<'_, Storage>,
) -> Result<String, String> {
    let out_path = timestamped_backup_path("sayit-config", "json");
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create backup directory: {}", e))?;
    }
    let payload = match selection.mode.as_str() {
        "full" => build_config_value(storage.inner()),
        "selected" => build_selected_config_value(storage.inner(), &selection)?,
        _ => return Err("Unsupported configuration export mode".to_string()),
    };
    let content = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
    fs::write(&out_path, content).map_err(|e| format!("Failed to write file: {}", e))?;
    let path = out_path.to_string_lossy().to_string();
    log::info!("Config backup exported to {}", path);
    Ok(path)
}

/// 打包过程中的一帧进度。`phase` 取值与前端 `BackupExportProgress['phase']` 对齐。
pub struct ArchiveProgress<'a> {
    pub phase: &'static str,
    pub current_file: Option<&'a str>,
    pub processed_files: u64,
    pub total_files: u64,
    pub processed_bytes: u64,
    pub total_bytes: u64,
    pub percent: f64,
}

/// 把配置（+ 按 scope 决定的历史 / 音频）打成 zip 写到 `output`。
///
/// 本地「全部数据」导出与 WebDAV 备份共用这一份实现，两者的差别只在落地位置和
/// 进度事件名。刻意不让 WebDAV 自己写一套打包：一旦两份代码分家，就会出现
/// 「本地导出能恢复、云端备份恢复不了」这种只在换机时才暴露的问题。
///
/// 先写 `.part` 再原子 rename，失败时清掉临时文件 —— 中断留下的半个 zip 不能
/// 长得像一个有效备份。
///
/// 总量（文件数 / 字节数）通过 `on_progress` 报出去，不作为返回值：调用方要的是
/// 打包完成后**zip 本身**的大小（用来算上传进度），那个得 stat 产物文件才知道。
pub fn write_backup_archive(
    storage: &Storage,
    scope: BackupScope,
    output: &Path,
    on_progress: &mut dyn FnMut(ArchiveProgress),
) -> Result<(), String> {
    let temp_path = PathBuf::from(format!("{}.part", output.to_string_lossy()));
    let audio_files = collect_audio_export_files(scope.include_audio);
    let total_files = audio_files.len() as u64;
    let total_bytes = audio_files.iter().map(|file| file.size).sum::<u64>();

    on_progress(ArchiveProgress {
        phase: "preparing",
        current_file: None,
        processed_files: 0,
        total_files,
        processed_bytes: 0,
        total_bytes,
        percent: 2.0,
    });

    let result = (|| -> Result<(), String> {
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create export directory: {}", e))?;
        }

        let payload = build_full_value(storage, scope);
        let json_str = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
        on_progress(ArchiveProgress {
            phase: "packingData",
            current_file: Some("backup.json"),
            processed_files: 0,
            total_files,
            processed_bytes: 0,
            total_bytes,
            percent: 5.0,
        });

        let file = fs::File::create(&temp_path).map_err(|e| format!("Failed to create file: {}", e))?;
        let mut zip = zip::ZipWriter::new(file);
        let text_opts = zip::write::FileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let audio_opts = zip::write::FileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        zip.start_file("backup.json", text_opts)
            .map_err(|e| e.to_string())?;
        zip.write_all(json_str.as_bytes())
            .map_err(|e| e.to_string())?;

        let mut processed_bytes = 0u64;
        let mut processed_files = 0u64;
        let mut last_emit = Instant::now() - Duration::from_secs(1);
        let mut buffer = vec![0u8; 256 * 1024];

        for audio in &audio_files {
            zip.start_file(format!("audio/{}", audio.name), audio_opts)
                .map_err(|e| e.to_string())?;
            let mut source = fs::File::open(&audio.path)
                .map_err(|e| format!("Failed to read audio file {}: {}", audio.name, e))?;

            loop {
                let read = source
                    .read(&mut buffer)
                    .map_err(|e| format!("Failed to read audio file {}: {}", audio.name, e))?;
                if read == 0 {
                    break;
                }
                zip.write_all(&buffer[..read]).map_err(|e| e.to_string())?;
                processed_bytes += read as u64;

                if last_emit.elapsed() >= Duration::from_millis(150) {
                    on_progress(ArchiveProgress {
                        phase: "packingAudio",
                        current_file: Some(&audio.name),
                        processed_files,
                        total_files,
                        processed_bytes,
                        total_bytes,
                        percent: audio_progress_percent(processed_bytes, total_bytes),
                    });
                    last_emit = Instant::now();
                }
            }

            processed_files += 1;
            on_progress(ArchiveProgress {
                phase: "packingAudio",
                current_file: Some(&audio.name),
                processed_files,
                total_files,
                processed_bytes,
                total_bytes,
                percent: audio_progress_percent(processed_bytes, total_bytes),
            });
        }

        on_progress(ArchiveProgress {
            phase: "finalizing",
            current_file: None,
            processed_files,
            total_files,
            processed_bytes,
            total_bytes,
            percent: 98.0,
        });
        zip.finish().map_err(|e| e.to_string())?;

        if output.exists() {
            fs::remove_file(output).map_err(|e| format!("Failed to replace the previous backup: {}", e))?;
        }
        fs::rename(&temp_path, output).map_err(|e| format!("Failed to finalize the backup file: {}", e))?;
        Ok(())
    })();

    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temp_path);
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn export_full(
    app: AppHandle,
    scope: BackupScope,
    storage: State<'_, Storage>,
) -> Result<String, String> {
    let output = timestamped_backup_path("sayit-backup", "zip");
    let out_path = output.to_string_lossy().to_string();

    // 失败分支也要报出总量，所以在回调里留一份最近值：write_backup_archive 出错时
    // 只给 Err(String)，拿不到它内部算出的 totals。
    let mut seen_totals = (0u64, 0u64);
    let export_result = {
        let progress_app = app.clone();
        let progress_path = out_path.clone();
        write_backup_archive(storage.inner(), scope, &output, &mut |progress| {
            seen_totals = (progress.total_files, progress.total_bytes);
            emit_export_progress(
                &progress_app,
                BackupExportProgress {
                    status: "running".to_string(),
                    phase: progress.phase.to_string(),
                    file_path: progress_path.clone(),
                    current_file: progress.current_file.map(ToString::to_string),
                    processed_files: progress.processed_files,
                    total_files: progress.total_files,
                    processed_bytes: progress.processed_bytes,
                    total_bytes: progress.total_bytes,
                    percent: progress.percent,
                    error: None,
                },
            );
        })
    };
    let (total_files, total_bytes) = seen_totals;

    match export_result {
        Ok(()) => {
            emit_export_progress(
                &app,
                BackupExportProgress {
                    status: "completed".to_string(),
                    phase: "completed".to_string(),
                    file_path: out_path.clone(),
                    current_file: None,
                    processed_files: total_files,
                    total_files,
                    processed_bytes: total_bytes,
                    total_bytes,
                    percent: 100.0,
                    error: None,
                },
            );
            log::info!(
                "Full backup exported to {} (history={} audio={})",
                out_path,
                scope.include_history,
                scope.include_audio
            );
            Ok(out_path)
        }
        Err(error) => {
            // 临时文件已由 write_backup_archive 清理。
            emit_export_progress(
                &app,
                BackupExportProgress {
                    status: "failed".to_string(),
                    phase: "failed".to_string(),
                    file_path: out_path,
                    current_file: None,
                    processed_files: 0,
                    total_files,
                    processed_bytes: 0,
                    total_bytes,
                    percent: 0.0,
                    error: Some(error.clone()),
                },
            );
            Err(error)
        }
    }
}

// ─── 导入 ───

fn content_fingerprint(content: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in content.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn read_config_file(in_path: &str) -> Result<(Value, String), String> {
    let content = fs::read_to_string(in_path).map_err(|e| format!("Failed to read file: {}", e))?;
    let data = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse the file; it may be damaged or have an invalid format: {}", e))?;
    Ok((data, content_fingerprint(&content)))
}

fn config_import_token(
    storage: &Storage,
    data: &Value,
    file_fingerprint: &str,
) -> Result<String, String> {
    if !is_selected_config(data) {
        return Ok(file_fingerprint.to_string());
    }

    let context = json!({
        "file": file_fingerprint,
        "customHotwordThemes": storage.get("customHotwordThemes", None),
        "customThemeActive": storage.get("customThemeActive", None),
        "textReplacements": storage.get("textReplacements", None),
        "promptPresets": storage.get("promptPresets", None),
    });
    let serialized = serde_json::to_string(&context)
        .map_err(|error| format!("Failed to prepare import validation data: {}", error))?;
    Ok(content_fingerprint(&serialized))
}

fn validate_config_collection(key: &str, items: &[Value]) -> Result<(), String> {
    let mut ids = HashSet::new();
    for (index, item) in items.iter().enumerate() {
        let object = item
            .as_object()
            .ok_or_else(|| format!("Item {} in {} has an invalid format", index + 1, key))?;
        let id = object
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("Item {} in {} is missing id", index + 1, key))?;

        // 内置 Prompt 的中英文自定义内容共存在 promptPresets 中，因此同一个稳定 id
        // 可以出现两次，但同一语言仍只能有一份。旧备份没有语言字段，只能是中文。
        let unique_id = if key == "promptPresets" && BUILTIN_PRESET_IDS.contains(&id) {
            let language = object
                .get("builtinPromptLanguage")
                .and_then(Value::as_str)
                .unwrap_or("zh-CN");
            if language != "zh-CN" && language != "en" {
                return Err(format!(
                    "Item {} in {} has an invalid language",
                    index + 1,
                    key
                ));
            }
            format!("{}:{}", id, language)
        } else {
            id.to_string()
        };
        if !ids.insert(unique_id) {
            return Err(format!("{} contains a duplicate id: {}", key, id));
        }

        let required_fields: &[&str] = match key {
            "promptPresets" => &["name", "systemPrompt"],
            "appPromptRules" => &["appId", "name"],
            _ => &[],
        };
        for field in required_fields {
            let valid = object
                .get(*field)
                .and_then(Value::as_str)
                .is_some_and(|value| !value.trim().is_empty());
            if !valid {
                return Err(format!(
                    "Item {} in {} is missing {}",
                    index + 1,
                    key,
                    field
                ));
            }
        }
    }
    Ok(())
}

fn full_config_preview_sections(data: &Value) -> Result<Vec<ConfigImportSectionPreview>, String> {
    let mut sections = Vec::new();

    if let Some(value) = data.get("appSettings") {
        let settings = value
            .as_object()
            .ok_or_else(|| "appSettings has an invalid format".to_string())?;
        if !settings.is_empty() {
            sections.push(ConfigImportSectionPreview {
                kind: "appSettings".to_string(),
                total: settings.len(),
                added: 0,
                updated: settings.len(),
                skipped: 0,
            });
        }
    }

    for (key, kind) in [
        ("promptPresets", "promptPresets"),
        ("appPromptRules", "appPromptRules"),
    ] {
        if let Some(value) = data.get(key) {
            let items = value
                .as_array()
                .ok_or_else(|| format!("{} has an invalid format", key))?;
            validate_config_collection(key, items)?;
            sections.push(ConfigImportSectionPreview {
                kind: kind.to_string(),
                total: items.len(),
                added: 0,
                updated: items.len(),
                skipped: 0,
            });
        }
    }

    if sections.is_empty() {
        return Err("The file contains no complete configuration to import".to_string());
    }
    Ok(sections)
}

#[tauri::command]
pub async fn inspect_config_import(
    in_path: String,
    storage: State<'_, Storage>,
) -> Result<ConfigImportPreview, String> {
    let (data, file_fingerprint) = read_config_file(&in_path)?;
    let version = check_kind_and_version(
        &data,
        "config",
        SELECTED_CONFIG_FORMAT_VERSION,
    )?;

    if is_selected_config(&data) {
        let plan = build_selected_import_plan(storage.inner(), &data)?;
        let import_token = config_import_token(storage.inner(), &data, &file_fingerprint)?;
        return Ok(ConfigImportPreview {
            scope: "selected".to_string(),
            format_version: version,
            import_token,
            sections: plan.sections,
            warnings: plan.warnings,
            requires_restart: true,
        });
    }

    if data.get("scope").is_some() || version > FORMAT_VERSION {
        return Err("Unsupported configuration scope or version".to_string());
    }

    let sections = full_config_preview_sections(&data)?;
    Ok(ConfigImportPreview {
        scope: "full".to_string(),
        format_version: version,
        import_token: file_fingerprint,
        sections,
        warnings: vec![ConfigImportWarning {
            code: "fullOverwrite".to_string(),
            current: None,
            limit: None,
        }],
        requires_restart: true,
    })
}

#[tauri::command]
pub async fn import_config(
    in_path: String,
    expected_import_token: String,
    storage: State<'_, Storage>,
) -> Result<ConfigImportResult, String> {
    let (data, file_fingerprint) = read_config_file(&in_path)?;
    let version = check_kind_and_version(
        &data,
        "config",
        SELECTED_CONFIG_FORMAT_VERSION,
    )?;

    if is_selected_config(&data) {
        let actual_token = config_import_token(storage.inner(), &data, &file_fingerprint)?;
        if actual_token != expected_import_token {
            return Err("The configuration file or local settings changed; select the file again and reconfirm".to_string());
        }

        let plan = build_selected_import_plan(storage.inner(), &data)?;
        let changed_sections = plan
            .sections
            .iter()
            .filter(|section| section.added + section.updated > 0)
            .map(|section| section.kind.clone())
            .collect::<Vec<_>>();
        storage
            .apply_config_transaction(
                &plan.app_settings,
                CONFIG_EXCLUDE,
                plan.prompt_presets.as_deref(),
                None,
            )
            .map_err(|error| format!("Failed to write configuration: {}", error))?;
        return Ok(ConfigImportResult {
            changed_sections,
            added: plan.added,
            updated: plan.updated,
            skipped: plan.skipped,
            requires_restart: true,
        });
    }

    if data.get("scope").is_some() || version > FORMAT_VERSION {
        return Err("Unsupported configuration scope or version".to_string());
    }
    full_config_preview_sections(&data)?;
    if file_fingerprint != expected_import_token {
        return Err("The configuration file changed; select it again and reconfirm".to_string());
    }

    // 完整配置导入排除 stats，避免覆盖本机使用统计。
    apply_config_part(storage.inner(), &data, CONFIG_EXCLUDE)?;
    Ok(ConfigImportResult {
        changed_sections: vec!["fullConfig".to_string()],
        added: 0,
        updated: 1,
        skipped: 0,
        requires_restart: true,
    })
}

#[tauri::command]
pub async fn import_full(in_path: String, storage: State<'_, Storage>) -> Result<(), String> {
    apply_full_backup(storage.inner(), &in_path)
}

/// 应用一个全量备份 zip。本地导入与 WebDAV 恢复共用。
///
/// 对缺失段是宽容的：只含配置的备份（WebDAV 默认档）不会带 `history` 字段，
/// 此时**保留本机现有历史**而不是清空 —— 见 `build_full_value` 里为什么不写空数组。
pub fn apply_full_backup(storage: &Storage, in_path: &str) -> Result<(), String> {
    let file = fs::File::open(in_path).map_err(|e| format!("Failed to open file: {}", e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("Not a valid backup archive: {}", e))?;

    // 1. 读取并校验 backup.json
    let mut json_str = String::new();
    {
        let mut entry = archive
            .by_name("backup.json")
            .map_err(|_| "The archive is missing backup.json and may not be a SayIt full backup".to_string())?;
        entry.read_to_string(&mut json_str).map_err(|e| e.to_string())?;
    }
    let data: Value = serde_json::from_str(&json_str).map_err(|e| format!("Failed to parse backup.json: {}", e))?;
    check_kind_and_version(&data, "full", FORMAT_VERSION)?;

    // 2. 释放音频文件到本机 audio 目录
    let adir = audio_dir();
    fs::create_dir_all(&adir).map_err(|e| format!("Failed to create audio directory: {}", e))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().to_string();
        if let Some(rest) = name.strip_prefix("audio/") {
            if rest.is_empty() || entry.is_dir() {
                continue;
            }
            let base = basename(rest);
            if base.is_empty() {
                continue;
            }
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).map_err(|e| e.to_string())?;
            fs::write(adir.join(&base), &buf).map_err(|e| format!("Failed to write audio file {}: {}", base, e))?;
        }
    }

    // 3. 应用配置部分（含 stats）
    apply_config_part(storage, &data, &[])?;

    // 4. 历史：重写音频路径后整表替换
    if let Some(history) = data.get("history").and_then(|v| v.as_array()) {
        let rewritten: Vec<Value> = history.iter().map(|rec| rewrite_audio_path(rec, &adir)).collect();
        storage
            .set("history", &Value::Array(rewritten))
            .map_err(|e| format!("Failed to write history: {}", e))?;
    }
    if let Some(arr) = data.get("manualCorrections") {
        if arr.is_array() {
            storage.set("manualCorrections", arr).map_err(|e| format!("Failed to write manual corrections: {}", e))?;
        }
    }
    if let Some(arr) = data.get("feedbackQueue") {
        if arr.is_array() {
            storage.set("feedbackQueue", arr).map_err(|e| format!("Failed to write feedback queue: {}", e))?;
        }
    }
    Ok(())
}

/// 导入完成后由前端调用，重启应用使内存中的设置/供应商状态全量重载。
#[tauri::command]
pub fn restart_app(app: tauri::AppHandle) {
    // 这次退出的意图是"重启回同一个版本"，不是更新。若让退出兜底安装跑起来，
    // 重启拉起来的会是正在被安装程序覆盖的 exe。
    crate::commands::system::suppress_exit_install();
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_storage(tag: &str) -> (Storage, PathBuf) {
        let dir = std::env::temp_dir().join(format!("sayit-backup-test-{}-{}", tag, uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let storage = Storage::new(dir.join("test.db")).unwrap();
        (storage, dir)
    }

    /// 不含历史时，`history` 字段必须**整个缺席**，不能是空数组。
    ///
    /// 这是「只备份配置」这一档能安全恢复的全部依据：`apply_full_backup` 用
    /// `if let Some(history)` 判断，字段缺席就保留本机历史，写成空数组则会把本机
    /// 历史整表清空 —— 一个「只想同步设置」的用户会因此丢掉全部记录，而且备份和
    /// 恢复都不会报任何错。
    #[test]
    fn config_only_backup_omits_history_instead_of_emptying_it() {
        let (storage, dir) = temp_storage("scope");
        storage
            .set(
                "history",
                &json!([{ "id": "h1", "timestamp": 1, "charCount": 3 }]),
            )
            .unwrap();

        let config_only = build_full_value(
            &storage,
            BackupScope {
                include_history: false,
                include_audio: false,
            },
        );
        assert!(
            config_only.get("history").is_none(),
            "history must be absent, got {:?}",
            config_only.get("history")
        );
        assert!(config_only.get("manualCorrections").is_none());
        assert!(config_only.get("feedbackQueue").is_none());
        // 配置部分照旧在，且 kind 仍是 full —— 用现有的导入路径就能恢复。
        assert_eq!(config_only.get("kind").and_then(Value::as_str), Some("full"));
        assert!(config_only.get("appSettings").is_some());
        assert_eq!(
            config_only.pointer("/scope/includeAudio").and_then(Value::as_bool),
            Some(false)
        );

        let with_history = build_full_value(
            &storage,
            BackupScope {
                include_history: true,
                include_audio: false,
            },
        );
        assert_eq!(
            with_history
                .get("history")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// 不含音频时不去扫 audio 目录，也就不会把别的备份的录音塞进来。
    #[test]
    fn audio_files_are_only_collected_when_requested() {
        assert!(collect_audio_export_files(false).is_empty());
    }

    #[test]
    fn prompt_overrides_allow_one_entry_per_builtin_language() {
        let items = vec![
            json!({
                "id": "intent",
                "name": "Intent cleanup",
                "systemPrompt": "中文自定义",
                "builtinPromptLanguage": "zh-CN",
            }),
            json!({
                "id": "intent",
                "name": "Intent cleanup",
                "systemPrompt": "English custom",
                "builtinPromptLanguage": "en",
            }),
        ];

        assert!(validate_config_collection("promptPresets", &items).is_ok());
    }

    #[test]
    fn prompt_overrides_reject_duplicate_builtin_language() {
        let items = vec![
            json!({ "id": "intent", "name": "Intent cleanup", "systemPrompt": "First" }),
            json!({
                "id": "intent",
                "name": "Intent cleanup",
                "systemPrompt": "Second",
                "builtinPromptLanguage": "zh-CN",
            }),
        ];

        assert!(validate_config_collection("promptPresets", &items).is_err());
    }
}
