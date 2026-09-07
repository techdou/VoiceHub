//! SayIt 0.1.9 runtime embedded in VoiceHub. See ../LICENSE and ../../UPSTREAM.md.
mod commands;
mod error_protocol;
mod locale;
mod storage;
mod window;
mod keyboard;
mod context;
mod inject;
mod providers;
mod models;
mod handler;

use tauri::Manager;
pub use handler::handler;

pub fn configure(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
}

pub fn setup(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let data = dirs::data_local_dir().ok_or("Local application data directory unavailable")?
        .join("app.soundbridge.windows/sayit");
    let storage = storage::Storage::new(data.join("sayit.db"))?;
    cleanup_retained_files(&storage, &data);
    models::custom::restore(storage.get("localAsr.customModelPath", None).as_str().filter(|s| !s.is_empty()).map(Into::into));
    if let Some(dir) = storage.get("localAsr.modelsDir", None).as_str().filter(|s| !s.is_empty()) {
        models::downloader::set_custom_models_dir(Some(dir.into()));
    }
    let ptt = storage.get("shortcutPTT", None).as_str().unwrap_or("ControlRight").to_owned();
    let hf = storage.get("shortcutHandsFree", None).as_str().unwrap_or("AltRight").to_owned();
    let ai = storage.get("shortcutToggleAi", None).as_str().unwrap_or("").to_owned();
    let idle = storage.get("localAsr.unloadIdleMinutes", None).as_u64().unwrap_or(0);
    app.manage(storage);
    app.manage(window::WindowState::new());
    app.manage(keyboard::KeyboardHookManager::new());
    app.manage(context::ContextDetector::new());
    app.state::<context::ContextDetector>().start_winevent_hook(app);
    if let Some(window) = app.get_webview_window("main") {
        #[cfg(target_os = "windows")]
        window.with_webview(|webview| {
            use webview2_com::Microsoft::Web::WebView2::Win32::{
                COREWEBVIEW2_PERMISSION_KIND_MICROPHONE, COREWEBVIEW2_PERMISSION_STATE_ALLOW,
            };
            use webview2_com::PermissionRequestedEventHandler;
            unsafe {
                if let Ok(core) = webview.controller().CoreWebView2() {
                    let handler = PermissionRequestedEventHandler::create(Box::new(|_, args| {
                        if let Some(args) = args {
                            let mut kind = Default::default();
                            args.PermissionKind(&mut kind)?;
                            if kind == COREWEBVIEW2_PERMISSION_KIND_MICROPHONE {
                                args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW)?;
                            }
                        }
                        Ok(())
                    }));
                    let mut token = Default::default();
                    if let Err(error) = core.add_PermissionRequested(&handler, &mut token) {
                        log::warn!("Microphone permission handler: {error}");
                    }
                }
            }
        })?;
    }
    app.state::<keyboard::KeyboardHookManager>().start(app, &ptt, &hf, &ai);
    commands::shortcuts::register_all_global_shortcuts(app, &app.state::<storage::Storage>());
    keyboard::spawn_health_watchdog();
    let overlay_app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(2));
        overlay_app.state::<window::WindowState>().prewarm_overlay(&overlay_app);
    });
    std::thread::spawn(move || {
        models::gguf_asr::init_backends();
        models::gguf_asr::spawn_idle_unloader(idle);
    });
    Ok(())
}

/// A bundled voice workspace must never install the standalone SayIt executable.
pub fn is_standalone_update_command(name: &str) -> bool {
    matches!(name, "download_update" | "install_downloaded_update" | "verify_update_package")
}

pub fn toggle_ai(app: &tauri::AppHandle) -> Result<bool, String> {
    commands::tray::toggle_tray_ai_enabled(app.clone(), app.state::<storage::Storage>())
}

fn cleanup_retained_files(storage: &storage::Storage, data: &std::path::Path) {
    for (folder, key, default) in [("audio", "audioRetentionDays", -1), ("logs", "logRetentionDays", 30)] {
        let days = storage.get(key, None).as_i64().unwrap_or(default);
        if days < 0 || (folder == "logs" && days == 0) { continue; }
        let cutoff = std::time::SystemTime::now().checked_sub(std::time::Duration::from_secs((days as u64).saturating_mul(86400)));
        let Some(cutoff) = cutoff else { continue };
        let Ok(entries) = std::fs::read_dir(data.join(folder)) else { continue };
        for entry in entries.flatten() {
            if entry.file_name() == "sayit.log" { continue; }
            if entry.file_type().is_ok_and(|kind| kind.is_file()) &&
                entry.metadata().and_then(|meta| meta.modified()).is_ok_and(|time| time < cutoff) {
                if let Err(error) = std::fs::remove_file(entry.path()) { log::warn!("Retention cleanup failed: {error}"); }
            }
        }
    }
}
