//! Tauri IPC 命令层：前端 ↔ 桥接服务。

use std::sync::Arc;

use tauri::{Manager, State};

use sb_core::buttons::RemoteButton;
use sb_core::gesture::Gesture;
use sb_core::settings::AppSettings;
use sb_core::statistics::UsageStatistics;

use crate::bridge::Bridge;

#[tauri::command]
pub fn get_settings(bridge: State<'_, Arc<Bridge>>) -> AppSettings {
    bridge.settings()
}

#[tauri::command]
pub fn save_settings(bridge: State<'_, Arc<Bridge>>, settings: AppSettings) -> Result<(), String> {
    bridge.set_gain(settings.gain_db);
    bridge.apply_settings(settings);
    Ok(())
}

#[tauri::command]
pub fn get_ble_snapshot(bridge: State<'_, Arc<Bridge>>) -> sb_windows::ble::BleSnapshot {
    bridge.ble.snapshot()
}

#[tauri::command]
pub fn list_paired_remotes(bridge: State<'_, Arc<Bridge>>) -> Vec<sb_windows::ble::PairedRemote> {
    bridge.ble.list_paired()
}

#[tauri::command]
pub fn connect_remote(bridge: State<'_, Arc<Bridge>>, device_id: String, name: String) -> Result<(), String> {
    {
        let mut settings = bridge.settings();
        settings.paired_device_id = Some(device_id.clone());
        settings.paired_device_name = Some(name);
        bridge.apply_settings(settings);
    }
    bridge.ble.connect(device_id);
    Ok(())
}

#[tauri::command]
pub fn disconnect_remote(bridge: State<'_, Arc<Bridge>>) {
    bridge.ble.disconnect();
}

#[tauri::command]
pub fn reconnect_remote(bridge: State<'_, Arc<Bridge>>) {
    bridge.ble.reconnect_now();
}

#[tauri::command]
pub fn list_audio_endpoints(bridge: State<'_, Arc<Bridge>>) -> Vec<sb_windows::AudioEndpoint> {
    bridge.audio.list_endpoints().unwrap_or_default()
}

#[tauri::command]
pub fn select_audio_endpoint(bridge: State<'_, Arc<Bridge>>, id: String, name: String) -> Result<(), String> {
    bridge
        .audio
        .select_endpoint(id)
        .map_err(|e| format!("选择「{name}」失败：{e}"))?;
    let mut settings = bridge.settings();
    settings.audio_endpoint_name = name;
    // 端点已真实打开：跳过 restore，避免释放-重开的互斥竞态窗口。
    bridge.apply_settings_with(settings, false);
    Ok(())
}

#[tauri::command]
pub fn get_statistics(bridge: State<'_, Arc<Bridge>>) -> UsageStatistics {
    bridge.statistics()
}

#[tauri::command]
pub fn get_history(bridge: State<'_, Arc<Bridge>>, limit: Option<usize>) -> Vec<sb_core::settings::VoiceSessionRecord> {
    bridge.store.load_history(limit.unwrap_or(200).min(1000))
}

#[tauri::command]
pub fn clear_history(bridge: State<'_, Arc<Bridge>>) -> Result<(), String> {
    bridge.store.clear_history().map_err(|e| e.to_string())
}

/// 绑定前台进程 → Profile（Smart Profiles）。
#[tauri::command]
pub fn bind_process_to_profile(
    bridge: State<'_, Arc<Bridge>>,
    process: String,
    profile_id: String,
) -> Result<(), String> {
    let mut settings = bridge.settings();
    settings
        .profiles
        .bind_process(&process, &profile_id)
        .map_err(|e| e.to_string())?;
    bridge.apply_settings(settings);
    Ok(())
}

#[tauri::command]
pub fn unbind_process(bridge: State<'_, Arc<Bridge>>, process: String) -> Result<(), String> {
    let mut settings = bridge.settings();
    settings.profiles.rules.process_bindings.remove(
        &sb_core::profiles::normalize_process_name(&process),
    );
    bridge.apply_settings(settings);
    Ok(())
}

#[tauri::command]
pub fn get_foreground_process() -> Option<String> {
    sb_windows::foreground::foreground_process_name()
}

/// 把某方案重置为出厂默认映射。
#[tauri::command]
pub fn reset_profile_to_default(bridge: State<'_, Arc<Bridge>>, profile_id: String) -> Result<(), String> {
    let mut settings = bridge.settings();
    let Some(profile) = settings
        .profiles
        .profiles
        .iter_mut()
        .find(|p| p.id == profile_id)
    else {
        return Err(format!("方案不存在：{profile_id}"));
    };
    profile.mapping = sb_core::mapping::default_mapping();
    bridge.apply_settings(settings);
    Ok(())
}

// ---------- 模拟遥控器 ----------

#[tauri::command]
pub fn simulate_button(bridge: State<'_, Arc<Bridge>>, button: String, gesture: String) -> Result<(), String> {
    let button = serde_json::from_value::<RemoteButton>(serde_json::Value::String(button))
        .map_err(|e| e.to_string())?;
    let gesture = match gesture.as_str() {
        "single" => Gesture::SingleClick,
        "double" => Gesture::DoubleClick,
        "long" => Gesture::LongPress,
        "repeat" => Gesture::Repeat,
        _ => return Err(format!("未知手势：{gesture}")),
    };
    bridge.simulate_gesture(button, gesture);
    Ok(())
}

#[tauri::command]
pub fn simulate_voice(bridge: State<'_, Arc<Bridge>>, duration_ms: Option<u64>) {
    bridge.simulate_voice(duration_ms.unwrap_or(2000).clamp(200, 10_000));
}

// ---------- 诊断 ----------

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticItem {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub status: String, // ok | warn | fail | info
}

#[tauri::command]
pub fn run_diagnostics(bridge: State<'_, Arc<Bridge>>, app: tauri::AppHandle) -> Vec<DiagnosticItem> {
    let mut items = Vec::new();

    // 蓝牙无线电。
    items.push(match sb_windows::radio::probe_radio() {
        Ok(true) => DiagnosticItem {
            id: "bluetooth".into(),
            title: "蓝牙无线电".into(),
            detail: "蓝牙适配器可用".into(),
            status: "ok".into(),
        },
        Ok(false) => DiagnosticItem {
            id: "bluetooth".into(),
            title: "蓝牙无线电".into(),
            detail: "未找到蓝牙适配器（或已关闭）".into(),
            status: "fail".into(),
        },
        Err(error) => DiagnosticItem {
            id: "bluetooth".into(),
            title: "蓝牙无线电".into(),
            detail: error,
            status: "warn".into(),
        },
    });

    // 已配对遥控器。
    let remotes = bridge.ble.list_paired();
    items.push(if remotes.is_empty() {
        DiagnosticItem {
            id: "paired_remote".into(),
            title: "遥控器配对".into(),
            detail: "系统未发现已配对的小米遥控器，请先在 Windows 蓝牙设置中配对".into(),
            status: "warn".into(),
        }
    } else {
        DiagnosticItem {
            id: "paired_remote".into(),
            title: "遥控器配对".into(),
            detail: format!("已配对：{}", remotes.iter().map(|r| r.name.clone()).collect::<Vec<_>>().join("、")),
            status: "ok".into(),
        }
    });

    // 当前连接。
    let snapshot = bridge.ble.snapshot();
    items.push(match snapshot.phase {
        sb_windows::ble::ConnectionPhase::Ready => DiagnosticItem {
            id: "connection".into(),
            title: "语音通道".into(),
            detail: format!("已连接 {}", snapshot.remote_name.clone().unwrap_or_default()),
            status: "ok".into(),
        },
        sb_windows::ble::ConnectionPhase::Stopped => DiagnosticItem {
            id: "connection".into(),
            title: "语音通道".into(),
            detail: "未连接（未选择遥控器）".into(),
            status: "info".into(),
        },
        phase => DiagnosticItem {
            id: "connection".into(),
            title: "语音通道".into(),
            detail: format!("{phase:?}{}", snapshot.last_error.map(|e| format!("：{e}")).unwrap_or_default()),
            status: "warn".into(),
        },
    });

    // VB-CABLE（双路：端点 + 驱动服务注册表）。
    let cable_status = {
        let data_dir = app.path().app_data_dir().unwrap_or_default();
        let endpoints = bridge.audio.list_endpoints().unwrap_or_default();
        let endpoint_present = endpoints.iter().any(|e| e.is_virtual_cable_candidate);
        crate::cable::status(&data_dir, endpoint_present)
    };
    items.push(if cable_status.installed() {
        DiagnosticItem {
            id: "virtual_cable".into(),
            title: "虚拟声卡".into(),
            detail: if cable_status.endpoint_present {
                "VB-CABLE 已就绪（CABLE Input 可见）".into()
            } else {
                "驱动已安装；CABLE Input 端点暂不可见（可能需要重启音频服务或系统）".into()
            },
            status: if cable_status.endpoint_present { "ok".into() } else { "warn".into() },
        }
    } else {
        DiagnosticItem {
            id: "virtual_cable".into(),
            title: "虚拟声卡".into(),
            detail: "未检测到 VB-CABLE：连接页可一键安装，或在语音工具里把录音设备设为 CABLE Output".into(),
            status: "warn".into(),
        }
    });

    // F5 吞键闸。
    items.push(if sb_windows::key_gate::is_installed() {
        DiagnosticItem {
            id: "key_gate".into(),
            title: "语音键拦截".into(),
            detail: "F5 吞键闸已安装".into(),
            status: "ok".into(),
        }
    } else {
        DiagnosticItem {
            id: "key_gate".into(),
            title: "语音键拦截".into(),
            detail: "F5 吞键闸未生效".into(),
            status: "warn".into(),
        }
    });

    // Provider 提示。
    let settings = bridge.settings();
    let provider_detail = match settings.provider.kind {
        sb_core::provider::ProviderKind::WeType => "微信输入法：请在其设置中开启语音快捷键 Ctrl+Win，并把录音设备设为 CABLE Output".into(),
        sb_core::provider::ProviderKind::Doubao => "豆包输入法：按住式触发，请确认其语音快捷键与声桥配置一致".into(),
        sb_core::provider::ProviderKind::SayIt => {
            let (vk, modifiers) = settings.provider.shortcut();
            if modifiers != 0 {
                format!(
                    "SayIt：录音设备设为 CABLE Output；触发键=组合键（vk 0x{vk:02X} + mods {modifiers}）——与 SayIt 的免提键保持一致即可被遥控器联动"
                )
            } else {
                "SayIt：录音设备设为 CABLE Output；当前触发键是单键（右 Alt/右 Ctrl）——SayIt 会忽略程序注入的单键，遥控器无法触发（手动按键可用）；遥控器联动请两边都改用组合键 Ctrl+Alt+H".into()
            }
        }
        sb_core::provider::ProviderKind::WinH => "Windows 听写（Win+H）：系统语音输入".into(),
        sb_core::provider::ProviderKind::Custom => "自定义语音工具".into(),
        sb_core::provider::ProviderKind::None => "未配置语音工具（仅测试音频链路）".into(),
    };
    items.push(DiagnosticItem {
        id: "provider".into(),
        title: "语音工具".into(),
        detail: provider_detail,
        status: "info".into(),
    });

    let _ = app;
    items
}

/// 虚拟声卡状态：端点 + 驱动服务 + 安装进度。
#[tauri::command]
pub fn check_virtual_cable(bridge: State<'_, Arc<Bridge>>, app: tauri::AppHandle) -> crate::cable::CableStatus {
    use tauri::Manager;
    let data_dir = app.path().app_data_dir().unwrap_or_default();
    let endpoint_present = bridge
        .audio
        .list_endpoints()
        .map(|endpoints| endpoints.iter().any(|e| e.is_virtual_cable_candidate))
        .unwrap_or(false);
    crate::cable::status(&data_dir, endpoint_present)
}

/// 一键安装：后台跑官方下载+校验+安装器（进度由 check_virtual_cable 轮询）。
#[tauri::command]
pub fn start_cable_install(app: tauri::AppHandle) -> Result<(), String> {
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    crate::cable::start_install(&data_dir)
}

#[tauri::command]
pub fn open_logs_folder(bridge: State<'_, Arc<Bridge>>, app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let log_dir = bridge.store.log_file().parent().map(|p| p.to_path_buf());
    if let Some(dir) = log_dir {
        let _ = std::fs::create_dir_all(&dir);
        let _ = app.opener().open_path(dir.to_string_lossy().to_string(), None::<String>);
    }
    Ok(())
}
