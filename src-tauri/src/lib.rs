//! 声枢 VoiceHub 宿主：托盘、单实例、自启、窗口管理、命令注册。

mod bridge;
mod cable;
mod commands;
mod store;
mod remote_voice;

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
    quit: "退出声枢",
};

const TRAY_EN: TrayTexts = TrayTexts {
    show: "Show settings",
    reconnect: "Reconnect remote",
    quit: "Quit VoiceHub",
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
    let ai = tauri::menu::MenuItem::with_id(app, "toggle-ai", "AI 整理 / AI cleanup", true, None::<&str>)?;
    tauri::menu::Menu::with_items(app, &[&show, &reconnect, &ai, &quit])
}

/// 语言变化后由 bridge 调用：重建托盘菜单。
pub fn refresh_tray_menu(app: &tauri::AppHandle) {
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

    /// 路由契约：`generate_handler![...]` 注册的每个命令必须出现在 HARDWARE_COMMANDS
    /// 里——漏加白名单的命令会被静默路由给 speech handler，返回 unknown command。
    /// 文本级校验（解析本文件源码的宏块），与 bridge.rs 的 UiEvent 契约测试同风格。
    #[test]
    fn hardware_router_matches_handler_registration() {
        let source = include_str!("lib.rs");
        // 标记拆开定义：完整字面量出现在本测试代码里会被 include_str 命中，
        // find 就永远定位到测试自身而不是真正的宏块。
        let open_marker = ["tauri::generate_", "handler!["].concat();
        let close_marker = ["]", ");"].concat();
        let start = source
            .find(&open_marker)
            .expect("generate_handler macro not found");
        let end = source[start..]
            .find(&close_marker)
            .map(|offset| start + offset)
            .expect("generate_handler macro not terminated");
        let block = &source[start..end];
        let registered: Vec<&str> = block
            .lines()
            .filter_map(|line| line.trim().strip_prefix("commands::"))
            .map(|name| name.trim_end_matches(','))
            .collect();
        assert!(!registered.is_empty(), "failed to parse generate_handler block");
        for name in &registered {
            assert!(
                HARDWARE_COMMANDS.contains(name),
                "command `{name}` registered in generate_handler but missing from HARDWARE_COMMANDS"
            );
        }
        for name in HARDWARE_COMMANDS {
            assert!(
                registered.contains(name),
                "command `{name}` in HARDWARE_COMMANDS but not registered in generate_handler"
            );
        }
    }
}

/// 简单落盘 logger：无外部依赖，追加写入 app_data 下的日志文件。
/// 没有它，全应用的 log::info!/warn!/error! 都是空操作（logger 从未注册），
/// 真机出问题零排障证据。跨天自动切换到当日文件（托盘常驻应用生命周期
/// 跨天很常见，否则日志一直写启动日文件，open_logs_folder 打开的当天文件
/// 反而没有最新日志）。
struct FileLogger {
    state: std::sync::Mutex<LoggerState>,
}

struct LoggerState {
    dir: std::path::PathBuf,
    day: String,
    file: std::fs::File,
}

impl LoggerState {
    fn open(dir: &std::path::Path, day: &str) -> Option<LoggerState> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(format!("soundbridge-{day}.log")))
            .ok()?;
        Some(LoggerState { dir: dir.to_path_buf(), day: day.to_string(), file })
    }
}

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        use std::io::Write;
        if let Ok(mut state) = self.state.lock() {
            let today = chrono::Local::now().format("%Y%m%d").to_string();
            if today != state.day {
                // 跨天：切到新文件。打开失败则继续写旧文件，日志不能丢。
                if let Some(next) = LoggerState::open(&state.dir, &today) {
                    let _ = state.file.flush();
                    *state = next; // 旧文件句柄随赋值 drop 关闭
                }
            }
            let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
            let _ = writeln!(
                state.file,
                "{timestamp} {:<5} {}: {}",
                record.level(),
                record.target(),
                record.args()
            );
        }
    }

    fn flush(&self) {
        use std::io::Write;
        if let Ok(mut state) = self.state.lock() {
            let _ = state.file.flush();
        }
    }
}

fn init_logging(app: &tauri::AppHandle) -> Option<()> {
    let dir = app
        .path()
        .app_data_dir()
        .ok()?;
    let store = Store::new(&dir);
    let logs_dir = store.log_file().parent()?.to_path_buf();
    std::fs::create_dir_all(&logs_dir).ok()?;
    let today = chrono::Local::now().format("%Y%m%d").to_string();
    let state = LoggerState::open(&logs_dir, &today)?;
    // log crate 的 std feature 被依赖树关掉了，set_boxed_logger 不可用；
    // 用 set_logger + Box::leak（logger 生命周期 = 进程生命周期，泄漏即设计）。
    let logger: &'static FileLogger = Box::leak(Box::new(FileLogger {
        state: std::sync::Mutex::new(state),
    }));
    log::set_logger(logger).ok()?;
    log::set_max_level(log::LevelFilter::Info);
    log::info!("logging initialized");
    Some(())
}

/// hardware 命令路由白名单：命中则走主应用 handler，否则 fall through 给
/// speech（voicehub-sayit）handler。两边各有一个 `get_settings`（payload 不同），
/// 路由靠本清单——新增 hardware 命令必须同时加进 `generate_handler![...]` 和这里。
/// 一致性由 `hardware_router_matches_handler_registration` 测试锁定。
const HARDWARE_COMMANDS: &[&str] = &[
    "get_settings",
    "save_settings",
    "get_ble_snapshot",
    "list_paired_remotes",
    "connect_remote",
    "disconnect_remote",
    "reconnect_remote",
    "list_audio_endpoints",
    "select_audio_endpoint",
    "get_statistics",
    "get_history",
    "clear_history",
    "bind_process_to_profile",
    "unbind_process",
    "get_foreground_process",
    "reset_profile_to_default",
    "simulate_button",
    "simulate_voice",
    "check_virtual_cable",
    "start_cable_install",
    "run_diagnostics",
    "open_logs_folder",
    "remote_voice_poll",
    "remote_voice_ack",
    "remote_voice_reject",
];

pub fn run() {
    // A shared WebView2 environment must use one set of background timer flags.
    if std::env::var_os("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").is_none() {
        std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
            "--disable-backgrounding-occluded-windows --disable-renderer-backgrounding --disable-background-timer-throttling");
    }
    voicehub_sayit::configure(tauri::Builder::default()
        // single-instance 必须第一个注册才能拦截二次启动。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // 二次启动：唤起已有主窗口。
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init()))
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
            init_logging(app.handle());
            voicehub_sayit::setup(app.handle())?;

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
            let menu = build_tray_menu(app.handle())?;
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
                    "toggle-ai" => { let _ = voicehub_sayit::toggle_ai(app); }
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
        .invoke_handler({
          let hardware: Box<dyn Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync> = Box::new(tauri::generate_handler![
            commands::get_settings,
            commands::remote_voice_poll,
            commands::remote_voice_ack,
            commands::remote_voice_reject,
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
            commands::check_virtual_cable,
            commands::start_cable_install,
            commands::run_diagnostics,
            commands::open_logs_folder,
          ]);
          let speech = voicehub_sayit::handler();
          move |invoke: tauri::ipc::Invoke<tauri::Wry>| {
            let command = invoke.message.command();
            if voicehub_sayit::is_standalone_update_command(command) {
                invoke.resolver.reject("Updates are managed by VoiceHub");
                return true;
            }
            if HARDWARE_COMMANDS.contains(&command) {
                hardware(invoke)
            } else { speech(invoke) }
          }
        })
        .run(tauri::generate_context!())
        .expect("error while running VoiceHub");
}
