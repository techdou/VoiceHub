//! 前台进程检测（Smart Profiles 匹配输入）。

use windows::core::PWSTR;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
use windows::Win32::Foundation::GetLastError;

/// 当前前台进程可执行完整路径；失败返回 None。
pub fn foreground_process_path() -> Option<String> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return None;
        }
        let mut pid = 0u32;
        let _ = windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 1024];
        let mut len = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        );
        let _ = GetLastError();
        if ok.is_err() {
            return None;
        }
        Some(String::from_utf16_lossy(&buffer[..len as usize]))
    }
}

/// 前台进程短名（小写、不带 .exe），Smart Profiles 直接匹配用。
pub fn foreground_process_name() -> Option<String> {
    foreground_process_path().map(|path| {
        let name = path.rsplit(['\\', '/']).next().unwrap_or(&path);
        name.strip_suffix(".exe").unwrap_or(name).to_ascii_lowercase()
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn foreground_query_does_not_crash_headless() {
        // 测试环境无前台窗口也应安全返回。
        let _ = super::foreground_process_name();
    }
}
