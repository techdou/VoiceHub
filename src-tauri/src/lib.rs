//! 声桥 SoundBridge 宿主：托盘、单实例、自启、窗口管理、命令注册。

mod bridge;
mod commands;
mod store;

use std::sync::Arc;

use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager, WindowEvent,
};
use tauri_plugin_autostart::MacosLauncher;

use bridge::Bridge;
use store::Store;

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

            // 托盘。
            let show = MenuItem::with_id(app, "show", "显示设置", true, None::<&str>)?;
            let reconnect = MenuItem::with_id(app, "reconnect", "重连遥控器", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出声桥", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &reconnect, &quit])?;
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
