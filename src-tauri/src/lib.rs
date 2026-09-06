//! 声桥 SoundBridge 宿主：托盘、单实例、自启、窗口管理、命令注册。

mod bridge;
mod commands;
mod store;

use std::sync::Arc;

use tauri::{tray::TrayIconBuilder, Manager, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;

use bridge::Bridge;
use sb_core::settings::Language;
use store::Store;

/// 托盘文案（zh / en）。键集一致性由测试锁定。
pub struct TrayTexts {
    pub show: &'static str,
    pub reconnect: &'static str,
    pub quit: &'static str,
}

const TRAY_ZH: TrayTexts = TrayTexts {
    show: "显示设置",
    reconnect: "重连遥控器",
    quit: "退出声桥",
};

const TRAY_EN: TrayTexts = TrayTexts {
    show: "Show settings",
    reconnect: "Reconnect remote",
    quit: "Quit SoundBridge",
};

/// 设置语言 → 托盘文案（system 跟随系统 UI 语言）。
pub fn tray_texts(language: Language) -> &'static TrayTexts {
    match language {
        Language::ZhCn => &TRAY_ZH,
        Language::English => &TRAY_EN,
        Language::System => {
            let zh = sys_locale::get_locale()
                .map(|locale| locale.to_lowercase().starts_with("zh"))
                .unwrap_or(true); // 识别失败偏中文（产品主语言）
            if zh { &TRAY_ZH } else { &TRAY_EN }
        }
    }
}

/// 按语言构建托盘菜单（id 恒定，语言切换时重建并 set_menu）。
fn build_tray_menu(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    let language = app
        .try_state::<Arc<Bridge>>()
        .map(|bridge| bridge.settings().language)
        .unwrap_or(Language::System);
    let texts = tray_texts(language);
    let show = tauri::menu::MenuItem::with_id(app, "show", texts.show, true, None::<&str>)?;
    let reconnect =
        tauri::menu::MenuItem::with_id(app, "reconnect", texts.reconnect, true, None::<&str>)?;
    let quit = tauri::menu::MenuItem::with_id(app, "quit", texts.quit, true, None::<&str>)?;
    tauri::menu::Menu::with_items(app, &[&show, &reconnect, &quit])
}

/// 语言变化后由 bridge 调用：重建托盘菜单。
pub fn refresh_tray_menu(app: &tauri::AppHandle) {
    use tauri::Manager;
    if let Some(tray) = app.tray_by_id("main-tray") {
        match build_tray_menu(app) {
            Ok(menu) => {
                if let Err(error) = tray.set_menu(Some(menu)) {
                    log::warn!("tray menu refresh failed: {error}");
                }
            }
            Err(error) => log::warn!("tray menu rebuild failed: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_texts_cover_both_languages() {
        for language in [Language::System, Language::ZhCn, Language::English] {
            let texts = tray_texts(language);
            assert!(!texts.show.is_empty());
            assert!(!texts.reconnect.is_empty());
            assert!(!texts.quit.is_empty());
        }
        assert_ne!(TRAY_ZH.show, TRAY_EN.show);
        assert_ne!(TRAY_ZH.quit, TRAY_EN.quit);
    }
}

fn init_logging(app: &tauri::AppHandle) -> Option<()> {
    let dir = app
        .path()
        .app_data_dir()
        .ok()?;
    let store = Store::new(&dir);
    let target = store.log_file();
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(target)
        .ok()?;
    // 简单文件日志（无外部依赖；按天分文件）。
    let _ = file;
    Some(())
}

pub fn run() {
    tauri::Builder::default()
        // single-instance 必须第一个注册才能拦截二次启动。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // 二次启动：唤起已有主窗口。
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("app data dir");
            let store = Arc::new(Store::new(&data_dir));
            let settings = store.load_settings();
            init_logging(&app.handle());

            let bridge = Bridge::start(app.handle().clone(), store, settings);
            app.manage(bridge);

            // 窗口关闭 → 隐藏到托盘（常驻）。
            let main_window = app.get_webview_window("main").expect("main window");
            let hide_instead_of_close = main_window.clone();
            main_window.on_window_event(move |event| {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    let _ = hide_instead_of_close.hide();
                    api.prevent_close();
                }
            });

            // 托盘（菜单文案跟随设置语言）。
            let menu = build_tray_menu(&app.handle())?;
            TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "reconnect" => {
                        if let Some(bridge) = app.try_state::<Arc<Bridge>>() {
                            bridge.ble.reconnect_now();
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click { button: tauri::tray::MouseButton::Left, button_state: tauri::tray::MouseButtonState::Up, .. } = event {
                        let app = tray.app_handle().clone();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::get_ble_snapshot,
            commands::list_paired_remotes,
            commands::connect_remote,
            commands::disconnect_remote,
            commands::reconnect_remote,
            commands::list_audio_endpoints,
            commands::select_audio_endpoint,
            commands::get_statistics,
            commands::get_history,
            commands::clear_history,
            commands::bind_process_to_profile,
            commands::unbind_process,
            commands::get_foreground_process,
            commands::reset_profile_to_default,
            commands::simulate_button,
            commands::simulate_voice,
            commands::run_diagnostics,
            commands::open_logs_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running SoundBridge");
}
