use std::io::Read;
use std::path::PathBuf;
use std::sync::RwLock;
use tauri::State;
use crate::storage::Storage;
use super::catalog::{LocalModelInfo, ModelInfo};

pub const ID: &str = "voicehub-custom-gguf";
static PATH: RwLock<Option<PathBuf>> = RwLock::new(None);
static CHANGE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn restore(path: Option<PathBuf>) {
    *PATH.write().unwrap_or_else(|e| e.into_inner()) = path;
}
pub fn path() -> Option<PathBuf> { PATH.read().unwrap_or_else(|e| e.into_inner()).clone() }

#[tauri::command]
pub fn custom_model_path() -> Option<String> { path().map(|p| p.to_string_lossy().into_owned()) }

fn validate_path(raw: &str) -> Result<PathBuf, String> {
    let path = std::fs::canonicalize(raw.trim()).map_err(|e| format!("Cannot open model: {e}"))?;
    if !path.is_file() || !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("gguf")) {
        return Err("Select a GGUF speech recognition model".into());
    }
    let mut header = [0; 4];
    std::fs::File::open(&path).and_then(|mut f| f.read_exact(&mut header)).map_err(|e| e.to_string())?;
    if &header != b"GGUF" { return Err("Invalid GGUF file header".into()); }
    Ok(path)
}

#[tauri::command]
pub async fn register_custom_model(storage: State<'_, Storage>, model_path: String, accelerator: Option<String>) -> Result<String, String> {
    let _change = CHANGE.lock().await;
    let selected = validate_path(&model_path)?;
    let old = path();
    let value = selected.to_string_lossy().into_owned();
    let result = tokio::task::spawn_blocking(move || {
        super::gguf_asr::unload();
        restore(Some(selected));
        super::gguf_asr::preload(ID, accelerator.as_deref().unwrap_or("auto"))
    }).await.map_err(|e| e.to_string()).and_then(|r| r);
    if let Err(error) = result { restore(old); super::gguf_asr::unload(); return Err(error); }
    // Commit selection only after the native engine has accepted the model.
    if let Err(error) = storage.select_custom_model(&value, ID) {
        restore(old);
        super::gguf_asr::unload();
        return Err(error.to_string());
    }
    Ok(value)
}

pub fn local_info() -> Option<LocalModelInfo> {
    let path = path()?;
    let size = path.metadata().ok()?.len();
    Some(LocalModelInfo { id: ID.into(), name: path.file_stem()?.to_string_lossy().into_owned(),
        model_type: "gguf".into(), total_size_bytes: size, path: path.to_string_lossy().into_owned(), complete: size > 4 })
}
pub fn model_info() -> Option<ModelInfo> {
    let info = local_info()?;
    Some(ModelInfo { id: info.id, name: info.name, description: "Custom local GGUF".into(),
        model_type: info.model_type, total_size_bytes: info.total_size_bytes,
        languages: vec![], sources: vec![], archive_url: None, speed: 0.0, accuracy: 0.0,
        recommended: false, memory_mb: 0, languages_label: String::new(), quant: String::new(), featured: true })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_custom_model_does_not_change_selection() {
        assert!(validate_path("missing-model.gguf").is_err());
        assert!(validate_path(file!()).is_err());
    }
}
