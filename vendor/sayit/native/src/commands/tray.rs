//! SayIt 自定义托盘菜单窗口。
//!
//! Windows 原生菜单有系统级最小宽度，缩短文案也不会继续变窄；这里改用一个预创建、
//! 隐藏的轻量 WebView 窗口。右键时只定位并显示，避免每次创建 WebView 的延迟。

use crate::storage::Storage;
use serde_json::json;
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, Position, Size, State,
    WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

pub const TRAY_MENU_LABEL: &str = "tray-menu";
const TRAY_MENU_WIDTH: f64 = 140.0;
const TRAY_MENU_HEIGHT: f64 = 112.0;

pub fn create_tray_menu_window(app: &AppHandle) -> tauri::Result<()> {
    if app.get_webview_window(TRAY_MENU_LABEL).is_some() {
        return Ok(());
    }

    let window = WebviewWindowBuilder::new(
        app,
        TRAY_MENU_LABEL,
        WebviewUrl::App("tray-menu.html".into()),
    )
    .title("SayIt")
    .inner_size(TRAY_MENU_WIDTH, TRAY_MENU_HEIGHT)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .focused(false)
    .visible(false)
    .build()?;

    // 原生菜单点击外部会自动收起；自定义窗口用失焦还原同样的交互。
    let window_to_hide = window.clone();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Focused(false)) {
            let _ = window_to_hide.hide();
        }
    });

    Ok(())
}

/// 在托盘图标旁边显示菜单。`position` 是 Tauri 给出的全局物理坐标。
pub fn show_tray_menu(app: &AppHandle, position: PhysicalPosition<f64>) {
    let Some(window) = app.get_webview_window(TRAY_MENU_LABEL) else {
        log::warn!("[tray-menu] window is missing");
        return;
    };

    let monitor = app
        .available_monitors()
        .unwrap_or_default()
        .into_iter()
        .find(|monitor| {
            let origin = monitor.position();
            let size = monitor.size();
            position.x >= origin.x as f64
                && position.x < (origin.x as f64 + size.width as f64)
                && position.y >= origin.y as f64
                && position.y < (origin.y as f64 + size.height as f64)
        })
        .or_else(|| app.primary_monitor().ok().flatten());

    let (left, top, right, bottom, scale) = monitor
        .as_ref()
        .map(|monitor| {
            let origin = monitor.position();
            let size = monitor.size();
            (
                origin.x as f64,
                origin.y as f64,
                origin.x as f64 + size.width as f64,
                origin.y as f64 + size.height as f64,
                monitor.scale_factor(),
            )
        })
        .unwrap_or((0.0, 0.0, 1920.0, 1080.0, 1.0));

    let width = TRAY_MENU_WIDTH * scale;
    let height = TRAY_MENU_HEIGHT * scale;
    let gap = 8.0 * scale;
    let edge = 8.0 * scale;
    let monitor_width = right - left;
    let monitor_height = bottom - top;

    // 兼容任务栏停靠在上、下、左、右。常见的底部任务栏让菜单右边缘稍微越过
    // 光标中心，与 Windows 原生菜单从托盘图标向左展开的视觉锚点一致。
    let (mut x, mut y) = if position.y >= top + monitor_height * 0.75 {
        (position.x - width + 22.0 * scale, position.y - height - gap)
    } else if position.y <= top + monitor_height * 0.25 {
        (position.x - width + 22.0 * scale, position.y + gap)
    } else if position.x <= left + monitor_width * 0.25 {
        (position.x + gap, position.y - height / 2.0)
    } else {
        (position.x - width - gap, position.y - height / 2.0)
    };
    x = x.clamp(left + edge, (right - width - edge).max(left + edge));
    y = y.clamp(top + edge, (bottom - height - edge).max(top + edge));

    let _ = window.set_size(Size::Logical(LogicalSize::new(
        TRAY_MENU_WIDTH,
        TRAY_MENU_HEIGHT,
    )));
    let _ = window.set_position(Position::Physical(PhysicalPosition::new(
        x.round() as i32,
        y.round() as i32,
    )));
    let _ = window.show();
    let _ = window.set_always_on_top(true);
    let _ = window.set_focus();

    let enabled = current_ai_enabled(app.state::<Storage>().inner());
    let _ = window.emit("tray-menu-open", json!({ "enabled": enabled }));
}

fn current_ai_enabled(storage: &Storage) -> bool {
    storage.get("aiEnabled", None).as_bool().unwrap_or(true)
}

fn hide_menu(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(TRAY_MENU_LABEL) {
        let _ = window.hide();
    }
}

fn show_main(app: &AppHandle) {
    hide_menu(app);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[tauri::command]
pub fn set_tray_ai_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    // 主窗口里的开关已经负责落库；这里只把最新状态推给独立的托盘菜单 WebView。
    let _ = app.emit_to(
        TRAY_MENU_LABEL,
        "tray-ai-state",
        json!({ "enabled": enabled }),
    );
    Ok(())
}

#[tauri::command]
pub fn get_tray_ai_enabled(storage: State<'_, Storage>) -> bool {
    current_ai_enabled(storage.inner())
}

#[tauri::command]
pub fn toggle_tray_ai_enabled(
    app: AppHandle,
    storage: State<'_, Storage>,
) -> Result<bool, String> {
    let next = !current_ai_enabled(storage.inner());
    storage
        .set("aiEnabled", &json!(next))
        .map_err(|error| error.to_string())?;

    // 主窗口 store 会同步内存与录音器缓存；托盘窗口也立即刷新自己的图标。
    let _ = app.emit("ai-cleanup-changed", json!({ "enabled": next }));
    let _ = app.emit_to(
        TRAY_MENU_LABEL,
        "tray-ai-state",
        json!({ "enabled": next }),
    );
    log::info!("[tray-menu] AI cleanup toggled to {next}");
    Ok(next)
}

#[tauri::command]
pub fn show_main_from_tray(app: AppHandle) {
    show_main(&app);
}

#[tauri::command]
pub fn hide_tray_menu(app: AppHandle) {
    hide_menu(&app);
}

#[tauri::command]
pub fn quit_from_tray(app: AppHandle) {
    app.exit(0);
}
