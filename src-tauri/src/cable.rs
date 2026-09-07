//! VB-CABLE 一键安装：嵌入 PowerShell 脚本（官方包下载 + SHA-256 固定
//! 校验 + UAC 自提升 + 官方安装器 /install），状态写 JSON 供轮询。
//! 检测双路：音频端点名（CABLE Input）+ VBAudioVACMME 驱动服务注册表键。

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

const INSTALL_SCRIPT: &str = include_str!("../tools/install-vbcable.ps1");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CableInstallState {
    pub state: String,
    pub detail: String,
    pub exit_code: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CableStatus {
    /// 端点枚举发现 CABLE Input。
    pub endpoint_present: bool,
    /// 注册表存在 VBAudioVACMME 服务（驱动已装）。
    pub service_present: bool,
    pub install_state: Option<CableInstallState>,
    /// 安装脚本是否正在跑（starting/installing 中）。
    pub busy: bool,
}

impl CableStatus {
    pub fn installed(&self) -> bool {
        self.endpoint_present || self.service_present
    }
}

fn state_dir(app_data: &Path) -> PathBuf {
    app_data.join("vb-cable")
}

fn script_path(app_data: &Path) -> PathBuf {
    state_dir(app_data).join("install-vbcable.ps1")
}

/// 把内置脚本落到磁盘（幂等）。
pub fn materialize_script(app_data: &Path) -> std::io::Result<PathBuf> {
    let dir = state_dir(app_data);
    std::fs::create_dir_all(&dir)?;
    let path = script_path(app_data);
    std::fs::write(&path, INSTALL_SCRIPT)?;
    Ok(path)
}

/// 启动安装（立即返回；进度由 read_status 轮询）。
pub fn start_install(app_data: &Path) -> Result<(), String> {
    let script = materialize_script(app_data).map_err(|e| e.to_string())?;
    // 先清掉旧终态，避免轮询读到上一次的结果。
    let _ = std::fs::remove_file(state_dir(app_data).join("install-state.json"));
    // 绝对路径调用：按名解析走 CreateProcess 搜索顺序（应用目录优先于
    // System32），有 binary planting 面。
    let powershell = std::env::var("SystemRoot")
        .map(|root| format!(r"{root}\System32\WindowsPowerShell\v1.0\powershell.exe"))
        .unwrap_or_else(|_| "powershell.exe".into());
    Command::new(powershell)
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script)
        .arg("-Install")
        .arg("-StateDirectory")
        .arg(state_dir(app_data))
        .spawn()
        .map_err(|e| format!("启动安装脚本失败：{e}"))?;
    Ok(())
}

/// 读 install-state.json（无则 None）。损坏 JSON 按 None 处理。
pub fn read_install_state(app_data: &Path) -> Option<CableInstallState> {
    let raw = std::fs::read_to_string(state_dir(app_data).join("install-state.json")).ok()?;
    #[derive(Deserialize)]
    struct Raw {
        #[serde(default)]
        state: String,
        #[serde(default)]
        detail: String,
        #[serde(default)]
        exit_code: i32,
    }
    let raw: Raw = serde_json::from_str(&raw).ok()?;
    Some(CableInstallState {
        state: raw.state,
        detail: raw.detail,
        exit_code: raw.exit_code,
    })
}

/// 注册表检测 VBAudioVACMME 驱动服务（平台层实现）。
pub fn driver_service_present() -> bool {
    voicehub_windows::registry::driver_service_present()
}

/// busy 判定（纯函数，单测覆盖）。
pub fn is_busy_state(state: &str) -> bool {
    matches!(
        state,
        "downloading" | "elevation_required" | "verified" | "signature_warning" | "installing"
    )
}

/// 汇总状态：端点检测由调用方（bridge/commands）传入，避免模块依赖音频运行时。
pub fn status(app_data: &Path, endpoint_present: bool) -> CableStatus {
    let install_state = read_install_state(app_data);
    let busy = install_state
        .as_ref()
        .map(|s| is_busy_state(&s.state))
        .unwrap_or(false);
    CableStatus {
        endpoint_present,
        service_present: driver_service_present(),
        install_state,
        busy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_states_cover_progress_phases() {
        for state in [
            "downloading",
            "elevation_required",
            "verified",
            "signature_warning",
            "installing",
        ] {
            assert!(is_busy_state(state), "{state} should be busy");
        }
        for state in ["installed", "error", "verification_failed", "installer_failed", "not_started"] {
            assert!(!is_busy_state(state), "{state} is terminal");
        }
    }

    #[test]
    fn embedded_script_present_with_pinned_hash() {
        // 内核要素防意外改坏：URL、SHA、安装参数、UAC 提升都在。
        assert!(INSTALL_SCRIPT.contains("VBCABLE_Driver_Pack45.zip"));
        assert!(INSTALL_SCRIPT.contains("b950e39f01af1d04ea623c8f6d8eb9b6ea5c477c637295fabf20631c85116bfb"));
        assert!(INSTALL_SCRIPT.contains("\"/install\""));
        assert!(INSTALL_SCRIPT.contains("-Verb RunAs"));
    }

    #[test]
    fn status_parses_state_file() {
        let dir = std::env::temp_dir().join(format!("vh-cable-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("vb-cable")).unwrap();
        std::fs::write(
            dir.join("vb-cable/install-state.json"),
            r#"{"state":"downloading","detail":"...","exit_code":0}"#,
        )
        .unwrap();
        let cable = status(&dir, false);
        assert!(cable.busy);
        assert_eq!(cable.install_state.as_ref().unwrap().state, "downloading");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupted_state_file_is_none() {
        let dir = std::env::temp_dir().join(format!("vh-cable-bad-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("vb-cable")).unwrap();
        std::fs::write(dir.join("vb-cable/install-state.json"), "not json").unwrap();
        assert!(read_install_state(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
