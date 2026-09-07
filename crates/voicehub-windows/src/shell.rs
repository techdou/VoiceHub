//! 打开应用/网页 + 已运行实例切换 + 鼠标点击注入。

use windows::core::{BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
    KEYEVENTF_SCANCODE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetForegroundWindow,
    ShowWindow, SW_RESTORE, SW_SHOWNORMAL,
};

use crate::Result;

struct EnumState {
    target_lower: String,
    found: Option<HWND>,
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let state = &mut *(lparam.0 as *mut EnumState);
    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }
    let mut pid = 0u32;
    let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == 0 {
        return BOOL(1);
    }
    if let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
        let mut buffer = [0u16; 1024];
        let mut len = buffer.len() as u32;
        if QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
        .is_ok()
        {
            let path = String::from_utf16_lossy(&buffer[..len as usize]).to_lowercase();
            // 按路径末段精确相等比较：ends_with 会把 "notsayit.exe"
            // 误配成目标 "sayit.exe"。
            let exe_name = path.rsplit('\\').next().unwrap_or(path.as_str());
            if exe_name == state.target_lower {
                state.found = Some(hwnd);
                return BOOL(0); // 停止枚举
            }
        }
    }
    BOOL(1)
}

/// 目标已在运行 → 前置其主窗口；返回是否切换成功。
fn focus_running_instance(target: &str) -> bool {
    // target 取可执行短名（小写）匹配路径尾部。
    let exe_name = target.rsplit(['\\', '/']).next().unwrap_or(target).to_lowercase();
    if exe_name.is_empty() {
        return false;
    }
    let mut state = EnumState {
        target_lower: exe_name,
        found: None,
    };
    unsafe {
        let _ = EnumWindows(
            Some(enum_proc),
            LPARAM(&mut state as *mut EnumState as isize),
        );
    }
    if let Some(hwnd) = state.found {
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            SetForegroundWindow(hwnd).as_bool()
        }
    } else {
        false
    }
}

/// 打开可执行 / URI / 文档；exe 目标优先切换已运行实例。
pub fn open_target(target: &str) -> Result<()> {
    let looks_like_exe = target.to_lowercase().ends_with(".exe");
    if looks_like_exe && focus_running_instance(target) {
        return Ok(());
    }
    let wide: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(wide.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    // 返回值 > 32 表示成功（Win32 ShellExecute 约定）。
    if result.0 as usize > 32 {
        Ok(())
    } else {
        Err(crate::PlatformError::Message(format!(
            "ShellExecuteW 失败（code {}）：{target}",
            result.0 as i32
        )))
    }
}

/// 在当前光标位置注入一次左键点击（“点击确认”动作用）。
pub fn left_click() -> Result<()> {
    let inputs = [
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dwFlags: MOUSEEVENTF_LEFTDOWN,
                    ..Default::default()
                },
            },
        },
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dwFlags: MOUSEEVENTF_LEFTUP,
                    ..Default::default()
                },
            },
        },
    ];
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent != 2 {
        return Err(crate::PlatformError::Message("鼠标点击注入失败".into()));
    }
    Ok(())
}

/// 触发系统截图：区域截图（Win+Shift+S）或全屏到剪贴板（PrtScn 扩展键）。
pub fn screenshot(region: bool) -> Result<()> {
    if region {
        return crate::send_input::tap(crate::send_input::KeyChord::new(
            0x53, // S
            voicehub_core::actions::MOD_SHIFT | voicehub_core::actions::MOD_WIN,
        ));
    }
    // PrintScreen：扩展键，扫描码 0x37，必须带 EXTENDEDKEY。
    let inputs: Vec<INPUT> = vec![ext_key(0x37, false), ext_key(0x37, true)];
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent != 2 {
        return Err(crate::PlatformError::Message("截图键注入失败".into()));
    }
    Ok(())
}

fn ext_key(scan: u16, up: bool) -> INPUT {
    use windows::Win32::UI::Input::KeyboardAndMouse::KEYEVENTF_EXTENDEDKEY;
    let mut flags = KEYEVENTF_SCANCODE | KEYEVENTF_EXTENDEDKEY;
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: Default::default(),
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}
