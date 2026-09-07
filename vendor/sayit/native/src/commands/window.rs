use serde_json::Value;
use tauri::{AppHandle, State};

use crate::window::WindowState;

// IMPORTANT: 创建 WebView 的命令必须是 async，避免 Windows 主线程死锁。
#[tauri::command]
pub async fn present_overlay(
    data: Value,
    app: AppHandle,
    window_state: State<'_, WindowState>,
) -> Result<u64, String> {
    Ok(window_state.present_overlay(&app, data))
}

#[tauri::command]
pub async fn show_overlay(
    app: AppHandle,
    window_state: State<'_, WindowState>,
) -> Result<u64, String> {
    Ok(window_state.show_overlay(&app))
}

#[tauri::command]
pub async fn hide_overlay(
    app: AppHandle,
    window_state: State<'_, WindowState>,
) -> Result<(), String> {
    window_state.hide_overlay(&app);
    Ok(())
}

#[tauri::command]
pub async fn update_overlay_state(
    data: Value,
    app: AppHandle,
    window_state: State<'_, WindowState>,
) -> Result<(), String> {
    window_state.update_overlay_state(&app, &data);
    Ok(())
}

#[tauri::command]
pub fn overlay_ready(app: AppHandle, window_state: State<'_, WindowState>) {
    window_state.overlay_ready(&app);
}

#[tauri::command]
pub fn overlay_render_ack(data: Value, window_state: State<'_, WindowState>) {
    window_state.record_render_ack(&data);
}

#[tauri::command]
pub fn get_overlay_health(
    show_id: u64,
    app: AppHandle,
    window_state: State<'_, WindowState>,
) -> Value {
    window_state.health_snapshot(&app, show_id)
}

/// 旧版轻量 ping/pong，保留用于历史日志兼容。
#[tauri::command]
pub fn overlay_pong(seq: u64) {
    crate::window::record_overlay_pong(seq);
}
