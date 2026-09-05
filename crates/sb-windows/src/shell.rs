//! ShellExecute 打开应用/网页 + 鼠标点击注入。

use windows::core::PCWSTR;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
    KEYEVENTF_SCANCODE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::Result;

/// 打开可执行 / URI / 文档（系统默认处理器）。
pub fn open_target(target: &str) -> Result<()> {
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
            sb_core::actions::MOD_SHIFT | sb_core::actions::MOD_WIN,
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
