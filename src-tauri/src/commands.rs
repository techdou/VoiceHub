//! Tauri IPC 命令层：前端 ↔ 桥接服务。

use std::sync::Arc;

use tauri::{Manager, State};

use voicehub_core::buttons::RemoteButton;
use voicehub_core::gesture::Gesture;
use voicehub_core::settings::AppSettings;
use voicehub_core::statistics::UsageStatistics;

use crate::bridge::Bridge;

#[tauri::command]
pub fn remote_voice_poll(bridge: State<'_, Arc<Bridge>>, client_id: String) -> Option<crate::remote_voice::Packet> {
    bridge.remote_voice.lock().unwrap_or_else(|e| e.into_inner()).attach(&client_id);
    bridge.poll_remote_voice()
}
#[tauri::command]
pub fn remote_voice_ack(bridge: State<'_, Arc<Bridge>>, id: u64) -> bool {
    bridge.remote_voice.lock().unwrap_or_else(|e| e.into_inner()).ack(id)
}
#[tauri::command]
pub fn remote_voice_reject(bridge: State<'_, Arc<Bridge>>, id: u64) {
    bridge.remote_voice.lock().unwrap_or_else(|e| e.into_inner()).release(id);
}

#[tauri::command]
pub fn get_settings(bridge: State<'_, Arc<Bridge>>) -> AppSettings {
    bridge.settings()
}

#[tauri::command]
pub fn save_settings(bridge: State<'_, Arc<Bridge>>, settings: AppSettings) -> Result<(), String> {
    bridge.apply_settings(settings)
}

#[tauri::command]
pub fn get_ble_snapshot(bridge: State<'_, Arc<Bridge>>) -> voicehub_windows::ble::BleSnapshot {
    bridge.ble.snapshot()
}

#[tauri::command]
pub fn list_paired_remotes(bridge: State<'_, Arc<Bridge>>) -> Vec<voicehub_windows::ble::PairedRemote> {
    bridge.ble.list_paired()
}

#[tauri::command]
pub fn connect_remote(bridge: State<'_, Arc<Bridge>>, device_id: String, name: String) -> Result<(), String> {
    {
        let mut settings = bridge.settings();
        settings.paired_device_id = Some(device_id.clone());
        settings.paired_device_name = Some(name);
        settings.onboarding_complete = true;
        bridge.apply_settings(settings)?;
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
pub fn list_audio_endpoints(bridge: State<'_, Arc<Bridge>>) -> Vec<voicehub_windows::AudioEndpoint> {
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
    bridge.apply_settings_with(settings, false)
}

#[tauri::command]
pub fn get_statistics(bridge: State<'_, Arc<Bridge>>) -> UsageStatistics {
    bridge.statistics()
}

#[tauri::command]
pub fn get_history(bridge: State<'_, Arc<Bridge>>, limit: Option<usize>) -> Vec<voicehub_core::settings::VoiceSessionRecord> {
    bridge.store.load_history(limit.unwrap_or(200).min(1000))
}

#[tauri::command]
pub fn clear_history(bridge: State<'_, Arc<Bridge>>) -> Result<(), String> {
    bridge.store.clear_history().map_err(|e| e.to_string())
}

/// 绑定前台进程 → Profile（Smart Profiles）。
#[tauri::command]
pub fn get_foreground_process() -> Option<String> {
    voicehub_windows::foreground::foreground_process_name()
}

/// 把按键映射重置为出厂默认。
#[tauri::command]
pub fn reset_mapping_to_default(bridge: State<'_, Arc<Bridge>>) -> Result<(), String> {
    let mut settings = bridge.settings();
    settings.mapping = voicehub_core::mapping::default_mapping();
    bridge.apply_settings(settings)
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
pub fn simulate_voice(bridge: State<'_, Arc<Bridge>>, duration_ms: Option<u64>, audio_b64: Option<String>) -> Result<(), String> {
    use base64::Engine;
    let pcm = if let Some(encoded) = audio_b64 {
        if encoded.len() > 13_000_000 { return Err("Test audio must be at most five minutes".into()); }
        let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).map_err(|e| e.to_string())?;
        if bytes.is_empty() || bytes.len() % 2 != 0 || bytes.len() > 16_000 * 2 * 300 {
            return Err("Expected 16 kHz PCM, at most five minutes".into());
        }
        Some(bytes.chunks_exact(2).map(|s| i16::from_le_bytes([s[0], s[1]])).collect())
    } else { None };
    bridge.simulate_voice(duration_ms.unwrap_or(2000).clamp(200, 10_000), pcm)
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
    items.push(match voicehub_windows::radio::probe_radio() {
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
        voicehub_windows::ble::ConnectionPhase::Ready => DiagnosticItem {
            id: "connection".into(),
            title: "语音通道".into(),
            detail: format!("已连接 {}", snapshot.remote_name.clone().unwrap_or_default()),
            status: "ok".into(),
        },
        voicehub_windows::ble::ConnectionPhase::Stopped => DiagnosticItem {
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
    items.push(if bridge.settings().provider.kind == voicehub_core::provider::ProviderKind::SayIt {
        DiagnosticItem { id: "virtual_cable".into(), title: "音频通道".into(),
            detail: "内嵌识别：遥控器 PCM 直接送入语音引擎".into(), status: "ok".into() }
    } else if cable_status.installed() {
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

    // F5 吞键闸（三态：未安装 / 常驻拦截中 / 时序兜底）。
    items.push(if voicehub_windows::key_gate::is_installed() {
        if voicehub_windows::key_gate::is_persistent_armed() {
            DiagnosticItem {
                id: "key_gate".into(),
                title: "语音键拦截".into(),
                detail: "已安装；遥控器在线，F5 常驻拦截中（含键盘 F5，刷新请用 Ctrl+R）".into(),
                status: "ok".into(),
            }
        } else {
            DiagnosticItem {
                id: "key_gate".into(),
                title: "语音键拦截".into(),
                detail: "已安装；当前为时序兜底模式（遥控器离线或拦截开关已关闭），语音键漏出仍可能刷新前台页面".into(),
                status: "info".into(),
            }
        }
    } else {
        DiagnosticItem {
            id: "key_gate".into(),
            title: "语音键拦截".into(),
            detail: "F5 吞键闸未生效（约 1 秒后自动重装；若持续未生效请重启声枢）".into(),
            status: "warn".into(),
        }
    });

    // Provider 提示。
    let settings = bridge.settings();
    let provider_detail = match settings.provider.kind {
        voicehub_core::provider::ProviderKind::SayIt => "声枢内嵌 SayIt：由录音会话直接调用当前语音引擎".into(),
        voicehub_core::provider::ProviderKind::Custom => "自定义语音工具：触发键与模式在连接页配置（外部输入法兼容模式）".into(),
        voicehub_core::provider::ProviderKind::None => "未配置语音工具（仅测试音频链路）".into(),
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
