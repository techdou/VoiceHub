use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};
use crate::storage::Storage;
use std::io::Write;
use std::sync::Mutex;
use base64::Engine;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

/// Persistent log file writer — opened once, reused across calls.
static LOG_FILE: std::sync::LazyLock<Mutex<Option<std::fs::File>>> =
    std::sync::LazyLock::new(|| {
        let file = open_log_file();
        Mutex::new(file)
    });

/// Max log file size before rotation (5 MB)
const MAX_LOG_SIZE: u64 = 5 * 1024 * 1024;
/// Number of rotated files to keep
const ROTATED_FILES_KEEP: usize = 3;

fn log_dir() -> std::path::PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("app.soundbridge.windows/sayit")
        .join("logs")
}

fn log_file_path() -> std::path::PathBuf {
    log_dir().join("sayit.log")
}

fn open_log_file() -> Option<std::fs::File> {
    let dir = log_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("[log] failed to create log dir {:?}: {}", dir, e);
        return None;
    }
    let path = log_file_path();
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()
}

fn rotate_if_needed(file: &mut Option<std::fs::File>) {
    let path = log_file_path();
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if size < MAX_LOG_SIZE {
        return;
    }

    // Windows 不允许重命名仍由本进程打开的日志文件。先释放句柄，否则轮转会
    // 静默失败，正式版长期运行后 sayit.log 会无限增长。
    *file = None;

    // Rotate: sayit.log -> sayit.1.log, sayit.1.log -> sayit.2.log, etc.
    let dir = log_dir();
    for i in (1..ROTATED_FILES_KEEP).rev() {
        let from = dir.join(format!("sayit.{}.log", i));
        let to = dir.join(format!("sayit.{}.log", i + 1));
        let _ = std::fs::rename(&from, &to);
    }
    let rotated = dir.join("sayit.1.log");
    let _ = std::fs::rename(&path, &rotated);
    *file = open_log_file();
}

pub fn write_log_line(line: &str) {
    // 用 unwrap_or_else 而非 unwrap()：即使这个 Mutex 曾在某次 panic 中被"污染"
    // （poisoned），日志功能也不能跟着永久失效——否则会形成"一次意外 panic 导致
    // 之后所有诊断日志都写不出来"的雪崩，恰恰是排查间歇性问题最怕遇到的情况。
    let mut guard = LOG_FILE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // Rotate check — reopen file if needed
    rotate_if_needed(&mut guard);
    if guard.is_none() {
        *guard = open_log_file();
    }
    if let Some(ref mut f) = *guard {
        let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let _ = writeln!(f, "[{}] {}", ts, line);
        let _ = f.flush();
    }
}

fn get_os_version() -> String {
    #[cfg(target_os = "windows")]
    {
        // 从注册表读取，不弹窗口
        use std::process::Command;
        Command::new("cmd")
            .args(["/C", "ver"])
            .creation_flags(0x08000000)
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s: String| s.trim().to_string())
            .unwrap_or_else(|| "Windows".to_string())
    }
    #[cfg(not(target_os = "windows"))]
    {
        whoami::distro()
    }
}

fn get_local_ip() -> String {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("8.8.8.8:80")?;
            s.local_addr()
        })
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

fn get_system_locale() -> String {
    // 用原生 Win32 API 取区域设置，避免拉起 PowerShell（冷启动 1-2 秒）。
    // GetUserDefaultLocaleName 返回形如 "zh-CN" 的 BCP-47 名称，与原 (Get-Culture).Name 一致。
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Globalization::GetUserDefaultLocaleName;
        // LOCALE_NAME_MAX_LENGTH = 85
        let mut buf = [0u16; 85];
        let len = unsafe { GetUserDefaultLocaleName(&mut buf) };
        if len > 1 {
            // 返回值含结尾的 NUL，切掉它。
            String::from_utf16_lossy(&buf[..(len as usize - 1)])
        } else {
            "unknown".to_string()
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("LANG").unwrap_or_else(|_| "unknown".to_string())
    }
}

fn get_total_memory_mb() -> u64 {
    // 用原生 GlobalMemoryStatusEx 取物理内存，避免拉起 PowerShell。
    // ullTotalPhys 为字节，/ 1MB 与原 TotalPhysicalMemory / 1MB 语义一致。
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        if unsafe { GlobalMemoryStatusEx(&mut status) }.is_ok() {
            status.ullTotalPhys / (1024 * 1024)
        } else {
            0
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        0
    }
}

#[derive(Serialize)]
pub struct ClientRuntimeInfo {
    #[serde(rename = "userId")]
    pub user_id: String,
    #[serde(rename = "userName")]
    pub user_name: String,
    #[serde(rename = "deviceId")]
    pub device_id: String,
    pub hostname: String,
    #[serde(rename = "clientVersion")]
    pub client_version: String,
    pub platform: String,
    #[serde(rename = "osVersion")]
    pub os_version: String,
    #[serde(rename = "localIp")]
    pub local_ip: String,
    #[serde(rename = "systemLocale")]
    pub system_locale: String,
    #[serde(rename = "cpuCores")]
    pub cpu_cores: usize,
    #[serde(rename = "memoryMb")]
    pub memory_mb: u64,
}

#[tauri::command]
pub fn get_client_runtime_info(storage: State<Storage>) -> Result<ClientRuntimeInfo, String> {
    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let user_name = whoami::username();

    // Persist device_id so it stays stable across restarts
    let existing = storage.get("deviceId", None);
    let device_id = if let Some(id) = existing.as_str() {
        if !id.is_empty() {
            id.to_string()
        } else {
            let new_id = format!("sayit-{}", uuid::Uuid::new_v4());
            let _ = storage.set("deviceId", &serde_json::json!(new_id));
            new_id
        }
    } else {
        let new_id = format!("sayit-{}", uuid::Uuid::new_v4());
        let _ = storage.set("deviceId", &serde_json::json!(new_id));
        new_id
    };

    Ok(ClientRuntimeInfo {
        user_id: user_name.clone(),
        user_name,
        device_id,
        hostname,
        client_version: env!("CARGO_PKG_VERSION").to_string(),
        platform: std::env::consts::OS.to_string(),
        os_version: get_os_version(),
        local_ip: get_local_ip(),
        system_locale: get_system_locale(),
        cpu_cores: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        memory_mb: get_total_memory_mb(),
    })
}

/// 系统**显示语言**，返回前端 `Locale` 用的标签（`zh-CN` / `en`）。
///
/// 前端 `ui.language = 'auto'` 时靠这个定语言。刻意不让前端用
/// `navigator.language` 自己判：那个跟的是 WebView 的语言，与托盘用的
/// `GetUserDefaultUILanguage` 可能给出不同结论，会出现「托盘中文、界面英文」。
/// 判定只保留一处（`locale::system_ui_lang`），两边都问它。
#[tauri::command]
pub fn get_system_ui_language() -> String {
    crate::locale::system_ui_lang().tag().to_string()
}

#[tauri::command]
pub fn get_auto_launch(app: AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_auto_launch(app: AppHandle, _enable: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let autostart = app.autolaunch();
    if _enable {
        autostart.enable().map_err(|e| e.to_string())
    } else {
        autostart.disable().map_err(|e| e.to_string())
    }
}

// ─── 自动更新 ───
//
// 分工：版本检查在前端（updateChecker.ts 拉 manifest 比版本号），Rust 只负责
// 下载、校验完整性、以及把安装程序拉起来。四个入口：
//   · download_update            下载并校验 SHA-512
//   · verify_update_package      启动时确认上次下载的包还在、还完整（省掉重复下载整包）
//   · install_downloaded_update  用户主动点「立即更新」时装，装完重新拉起
//   · install_pending_update_on_exit  用户没点就直接退出时，在退出路径上静默装掉
//
// 最后那个是"强制更新"真正落地的地方：只靠用户点图标，不点的人永远留在旧版。

/// 安装程序是否已经被拉起过。
///
/// 用户主动安装与退出兜底安装共用同一个 spawn，必须互斥：install_downloaded_update
/// 自己就会 app.exit(0)，那次退出同样会走 RunEvent::Exit，不拦住就会起两个安装程序
/// 互相抢文件锁。suppress_exit_install() 也靠它屏蔽掉"重启回同一版本"那种退出。
static INSTALLER_SPAWNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 让本次退出不要触发兜底安装。用于导入配置后的重启：用户要的是重启回同一个版本，
/// 不是更新；若在这里把新版装下去，重启拉起来的会是正在被覆盖的 exe。
pub fn suppress_exit_install() {
    INSTALLER_SPAWNED.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// 计算文件 SHA-512 并输出 **Base64** —— 与 manifest 的 sha512 字段同一种编码。
/// （gen-latest-yml.ps1 把 Get-FileHash 的 hex 转成了 Base64，别按 hex 去比。）
fn file_sha512_base64(path: &std::path::Path) -> Result<String, String> {
    use sha2::{Digest, Sha512};
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("Failed to open the installer file: {}", e))?;
    let mut hasher = Sha512::new();
    // 安装包有几十 MB，分块读，别整个塞进内存
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = std::io::Read::read(&mut file, &mut buffer)
            .map_err(|e| format!("Failed to read the installer file: {}", e))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(hasher.finalize()))
}

/// 确认磁盘上那个安装包仍然可用（存在 + 哈希对得上）。
///
/// 启动时用它决定「上次下载的包还能不能直接装」。没有这一步，"下载完等用户点"
/// 会让每次开机都把整包重下一遍 —— 旧流程下载完立刻安装，所以从来没暴露过。
#[tauri::command]
pub fn verify_update_package(file_path: String, sha512: Option<String>) -> Result<bool, String> {
    let path = std::path::Path::new(&file_path);
    if !path.is_file() {
        return Ok(false);
    }
    match sha512.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        Some(expected) => Ok(file_sha512_base64(path)? == expected),
        // 没记哈希的包（本次改造之前下载的）只能确认文件在
        None => Ok(true),
    }
}

#[derive(Serialize, Clone)]
struct UpdateDownloadProgress {
    #[serde(rename = "downloadedBytes")]
    downloaded_bytes: u64,
    #[serde(rename = "totalBytes")]
    total_bytes: u64,
    percent: f64,
    status: String,
    error: Option<String>,
}

fn emit_update_progress(app: &AppHandle, downloaded: u64, total: u64, status: &str, error: Option<&str>) {
    let percent = if total > 0 {
        (downloaded as f64 / total as f64 * 100.0).min(100.0)
    } else {
        0.0
    };
    let _ = app.emit(
        "update-download-progress",
        UpdateDownloadProgress {
            downloaded_bytes: downloaded,
            total_bytes: total,
            percent,
            status: status.to_string(),
            error: error.map(String::from),
        },
    );
}

/// 下载更新安装包到临时目录，下载过程中通过 update-download-progress 事件上报真实字节进度。
/// 传了 sha512（manifest 里的 Base64）就在落盘后校验，不通过直接删掉并报错。
#[tauri::command]
pub async fn download_update(app: AppHandle, url: String, sha512: Option<String>) -> Result<String, String> {
    use futures_util::StreamExt;
    use std::io::Write;

    // 必须带 User-Agent：生产环境 AWS WAF 的 NoUserAgent_HEADER 规则会拦截无 UA 的请求（403）
    let client = reqwest::Client::builder()
        .user_agent(concat!("SayIt/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("Failed to initialize download client: {}", e))?;

    emit_update_progress(&app, 0, 0, "downloading", None);

    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(300))
        .send()
        .await
        .map_err(|e| {
            let msg = format!("Download failed: {}", e);
            emit_update_progress(&app, 0, 0, "failed", Some(&msg));
            msg
        })?;

    if !resp.status().is_success() {
        let msg = format!("Download failed: HTTP {}", resp.status());
        emit_update_progress(&app, 0, 0, "failed", Some(&msg));
        return Err(msg);
    }

    let total = resp.content_length().unwrap_or(0);

    // 从 URL 提取文件名
    let filename = url.split('/').last().unwrap_or("SayIt-Setup.exe").to_string();
    let temp_dir = std::env::temp_dir().join("sayit-update");
    std::fs::create_dir_all(&temp_dir).map_err(|e| format!("Failed to create temporary directory: {}", e))?;
    let file_path = temp_dir.join(&filename);

    let mut file = std::fs::File::create(&file_path)
        .map_err(|e| format!("Failed to create file: {}", e))?;

    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();
    let mut last_emit = std::time::Instant::now();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            let msg = format!("Download interrupted: {}", e);
            emit_update_progress(&app, downloaded, total, "failed", Some(&msg));
            msg
        })?;
        file.write_all(&chunk).map_err(|e| format!("Failed to write file: {}", e))?;
        downloaded += chunk.len() as u64;

        // 每 200ms 发送一次进度，避免刷屏
        if last_emit.elapsed().as_millis() >= 200 {
            emit_update_progress(&app, downloaded, total, "downloading", None);
            last_emit = std::time::Instant::now();
        }
    }

    file.flush().map_err(|e| format!("Failed to flush file: {}", e))?;
    drop(file);

    // 校验放在"completed"之前：前端收到 completed 就会把这个包记成待安装，
    // 先报完成再发现哈希不对，等于让一个坏包进入待安装状态。
    if let Some(expected) = sha512.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        let actual = file_sha512_base64(&file_path).map_err(|e| {
            emit_update_progress(&app, downloaded, total, "failed", Some(&e));
            e
        })?;
        if actual != expected {
            // 坏包必须删掉：它会在临时目录里等到用户点更新，留着的话下次启动
            // verify_update_package 只看文件名，会把它当成"已下载"直接拿去装。
            let _ = std::fs::remove_file(&file_path);
            let msg = "Update package integrity check failed (SHA-512 mismatch)".to_string();
            write_log_line("[update] downloaded package failed SHA-512 verification, discarded");
            emit_update_progress(&app, downloaded, total, "failed", Some(&msg));
            return Err(msg);
        }
    }

    // 新包已经校验通过，把同目录里其它版本的安装包删掉。
    // 旧流程下载完立刻安装，包只存在几秒；现在它按设计等到用户点更新或退出时才用，
    // 于是每升一次版就在 %TEMP% 里多躺一个十几 MB 的安装包，没人清。
    prune_stale_packages(&temp_dir, &filename);

    emit_update_progress(&app, downloaded, total.max(downloaded), "completed", None);

    Ok(file_path.to_string_lossy().to_string())
}

/// 删掉更新目录里除 keep 之外的所有文件。
///
/// 只在新包**校验通过之后**调用：下载失败时磁盘上那个旧包可能还是待安装的有效包，
/// 提前清理等于把用户已经下好的更新弄丢，还得重下一遍。
/// 删不掉（被占用等）只记日志不报错 —— 清理失败不该让一次成功的下载变成失败。
fn prune_stale_packages(dir: &std::path::Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut removed = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let matches_keep = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n == keep);
        if matches_keep {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => removed += 1,
            Err(e) => write_log_line(&format!(
                "[update] could not remove the stale package {}: {}",
                path.display(),
                e
            )),
        }
    }
    if removed > 0 {
        write_log_line(&format!("[update] removed {} stale update package(s)", removed));
    }
}

/// 拉起 NSIS 安装程序（/S 静默）。
///
/// 静默模式下 NSIS 不提供"安装完成后运行"，所以自己接管：起一个不依赖当前进程存活的
/// "看门人" cmd 进程，等安装进程真正退出（文件覆盖完成）后再决定要不要拉起新 exe。
/// 必须等旧进程完全退出才能覆盖 exe（Windows 文件锁），所以看门人要活在本进程之外。
///
/// 关键：不要把带引号、含 && 的复合命令直接塞给 `cmd /C` —— Rust 会把这个含空格的
/// 参数整体再套引号并把内部 " 转义成 \"，而 cmd.exe 不认识 \" 转义，路径会被啃坏
/// （历史 bug：装完自动重启报「找不到 \ 文件」）。脚本文件里的引号是文件字面量，
/// 不经过参数转义层；给 cmd /C 传单个脚本路径是 cmd 唯一能干净处理的情形。
///
/// relaunch=false 会装完就结束、不碰应用。目前两个调用方都传 true（用户点「立即安装」
/// 和退出兜底安装都要让用户看到新版本的关于页），参数保留是因为"装完别拉起来"这个选项
/// 一旦需要就必须立刻可用 —— 关机前退出那种场景下拉起应用是明确的错误行为。
#[cfg(target_os = "windows")]
fn spawn_installer(installer_path: &str, relaunch: bool) -> Result<(), String> {
    use std::process::Command;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let relaunch_line = if relaunch {
        let current_exe = std::env::current_exe()
            .map_err(|e| format!("Failed to get the current executable path: {}", e))?;
        // 同一路径：currentUser 安装模式下更新会装回原目录。--open-about 跳关于页，
        // 让用户能确认更新真的生效了。
        format!("start \"\" \"{}\" --open-about\r\n", current_exe.to_string_lossy())
    } else {
        String::new()
    };

    //   - ping 兜 ~1s，等旧进程完全退出，避免安装程序覆盖 exe 时撞文件锁
    //   - start /wait 等静默安装结束
    //   - del "%~f0" 让脚本运行完自删
    let script = format!(
        "@echo off\r\n\
         ping -n 2 127.0.0.1 >nul\r\n\
         start /wait \"\" \"{installer}\" /S\r\n\
         {relaunch}del \"%~f0\"\r\n",
        installer = installer_path,
        relaunch = relaunch_line,
    );
    let script_path = std::env::temp_dir().join("sayit-update-relaunch.bat");
    std::fs::write(&script_path, script)
        .map_err(|e| format!("Failed to write the update restart script: {}", e))?;

    Command::new("cmd")
        .args(["/C", &script_path.to_string_lossy()])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("Failed to start the installer watchdog: {}", e))?;
    Ok(())
}

/// 用户主动点「立即更新」：装完重新拉起应用。
#[tauri::command]
pub fn install_downloaded_update(file_path: String, relaunch: bool, app: AppHandle) -> Result<(), String> {
    let path = std::path::Path::new(&file_path);
    if !path.exists() {
        return Err("The installer file does not exist".to_string());
    }

    #[cfg(target_os = "windows")]
    {
        use std::sync::atomic::Ordering;
        // 抢在 app.exit(0) 之前置位：那次退出同样会走 RunEvent::Exit，
        // 退出兜底安装必须让开，否则两个安装程序抢同一个 exe。
        INSTALLER_SPAWNED.store(true, Ordering::SeqCst);
        if let Err(e) = spawn_installer(&file_path, relaunch) {
            INSTALLER_SPAWNED.store(false, Ordering::SeqCst);
            return Err(e);
        }
        write_log_line(&format!("[update] installer launched by user (relaunch={})", relaunch));
    }

    #[cfg(not(target_os = "windows"))]
    {
        return Err("Automatic installation is not supported on this platform".to_string());
    }

    // 退出当前应用，让安装程序能够覆盖 exe 文件
    app.exit(0);
    Ok(())
}

/// 退出时的兜底安装，由 main.rs 的 RunEvent::Exit 调用。
///
/// 用户没点「立即更新」就直接关掉了 SayIt —— 那就在退出路径上装掉。
/// 少了这一步，"发现新版就更新"完全依赖用户去点那个图标。
///
/// 装完会把应用重新拉起来并跳到关于页（`relaunch=true`）。这一条 2026-08 改过方向：
/// 原本传 false，理由是"用户是要关掉它，装完又拉起来像关不掉"。改回 true 的理由是
/// 升级完全无声时用户根本不知道版本变了，而这个"又自己开了"每个版本最多发生一次
/// （pendingUpdate 装完即清），代价比"永远不知道更新了什么"小。取舍记在
/// .kiro/decisions.md。
///
/// 只做存在性检查，不重算哈希：退出路径上不能卡几百毫秒，完整性在下载时已经验过。
pub fn install_pending_update_on_exit(app: &AppHandle) {
    use std::sync::atomic::Ordering;
    use tauri::Manager;

    // swap 而不是 load + store：退出事件理论上只来一次，但这里是唯一没有其它
    // 互斥保护的路径，用原子交换把"检查过了"和"占位"合成一步。
    if INSTALLER_SPAWNED.swap(true, Ordering::SeqCst) {
        return;
    }

    let storage: State<Storage> = app.state();
    let pending = storage.get("pendingUpdate", None);
    let file_path = match pending.get("filePath").and_then(|v| v.as_str()) {
        Some(value) if !value.is_empty() => value.to_string(),
        _ => return,
    };
    if !std::path::Path::new(&file_path).is_file() {
        return;
    }

    #[cfg(target_os = "windows")]
    {
        // relaunch=true：装完把应用拉回来并跳到关于页。见本函数上方注释里的取舍说明。
        match spawn_installer(&file_path, true) {
            Ok(()) => write_log_line("[update] pending update is being installed on exit (will relaunch)"),
            Err(e) => write_log_line(&format!("[update] exit-path install failed: {}", e)),
        }
    }

    #[cfg(not(target_os = "windows"))]
    let _ = file_path;
}

#[tauri::command]
pub fn append_debug_log(payload: Value) -> Result<(), String> {
    // Format a compact single-line representation for the log file
    let line = match payload {
        Value::Object(ref map) => {
            let kind = map.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
            match kind {
                "runtime" => {
                    let level = map.get("level").and_then(|v| v.as_str()).unwrap_or("info");
                    let source = map.get("source").and_then(|v| v.as_str()).unwrap_or("");
                    let message = map.get("message").and_then(|v| v.as_str()).unwrap_or("");
                    let detail = map.get("detail");
                    if let Some(d) = detail {
                        format!("[{}] [{}] {} {}", level.to_uppercase(), source, message, d)
                    } else {
                        format!("[{}] [{}] {}", level.to_uppercase(), source, message)
                    }
                }
                "session_start" => {
                    let sid = map.get("sessionId").and_then(|v| v.as_str()).unwrap_or("?");
                    format!("[SESSION] start id={}", sid)
                }
                "session_end" => {
                    let sid = map.get("sessionId").and_then(|v| v.as_str()).unwrap_or("?");
                    let dur = map.get("durationMs").and_then(|v| v.as_i64()).unwrap_or(0);
                    let msgs = map.get("messageCount").and_then(|v| v.as_i64()).unwrap_or(0);
                    format!("[SESSION] end id={} duration={}ms messages={}", sid, dur, msgs)
                }
                "ws_message" => {
                    let dir = map.get("direction").and_then(|v| v.as_str()).unwrap_or("?");
                    let typ = map.get("type").and_then(|v| v.as_str()).unwrap_or("?");
                    let data = map.get("data");
                    if let Some(d) = data {
                        format!("[WS] {} {} {}", dir, typ, d)
                    } else {
                        format!("[WS] {} {}", dir, typ)
                    }
                }
                _ => {
                    serde_json::to_string(&payload).unwrap_or_else(|_| format!("{:?}", payload))
                }
            }
        }
        _ => {
            serde_json::to_string(&payload).unwrap_or_else(|_| format!("{:?}", payload))
        }
    };

    write_log_line(&line);
    Ok(())
}

#[tauri::command]
pub fn save_audio_to_downloads(base64_data: String, filename: String) -> Result<String, String> {
    let downloads = dirs::download_dir()
        .ok_or_else(|| "Could not locate the Downloads directory".to_string())?;

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&base64_data)
        .map_err(|e| format!("Failed to decode base64 audio: {}", e))?;

    let dest = downloads.join(&filename);
    std::fs::write(&dest, &bytes).map_err(|e| format!("Failed to write file: {}", e))?;

    let path_str = dest.to_string_lossy().to_string();
    write_log_line(&format!("[INFO] [audio] Audio saved to {}", path_str));
    Ok(path_str)
}

/// 打开文件所在的文件夹（并选中文件）
#[tauri::command]
pub fn reveal_file_in_folder(file_path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        // explorer /select, 会打开文件夹并高亮选中文件
        std::process::Command::new("explorer")
            .arg("/select,")
            .arg(&file_path)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(&file_path)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        let path = std::path::PathBuf::from(&file_path);
        let dir = path.parent().unwrap_or(&path);
        std::process::Command::new("xdg-open")
            .arg(dir.to_string_lossy().to_string())
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }
    Ok(())
}

/// 直接打开指定文件夹。
#[tauri::command]
pub fn open_folder(folder_path: String) -> Result<(), String> {
    let path = std::path::PathBuf::from(&folder_path);
    if !path.is_dir() {
        return Err(format!("Folder does not exist: {}", folder_path));
    }

    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer")
        .arg(&path)
        .spawn()
        .map_err(|e| format!("Failed to open folder: {}", e))?;

    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg(&path)
        .spawn()
        .map_err(|e| format!("Failed to open folder: {}", e))?;

    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(&path)
        .spawn()
        .map_err(|e| format!("Failed to open folder: {}", e))?;

    Ok(())
}
