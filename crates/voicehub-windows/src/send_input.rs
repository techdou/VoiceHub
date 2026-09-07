//! SendInput 按键注入：快捷键组合、媒体键、音量键。
//!
//! 全部注入带 KEYEVENTF_SCANCODE（LL 钩子按 INJECTED 标记放行，
//! 避免自注事件被 key_gate 误吞）。

use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MAPVK_VK_TO_VSC_EX,
    VIRTUAL_KEY,
};

use crate::Result;

use voicehub_core::actions::{MediaKeyCode, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN};

/// 一个待注入的组合键。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyChord {
    pub vk: u16,
    pub modifiers: u8,
}

impl KeyChord {
    pub fn new(vk: u16, modifiers: u8) -> Self {
        Self { vk, modifiers }
    }
}

fn modifier_vks(modifiers: u8) -> Vec<u16> {
    let mut vks = Vec::new();
    if modifiers & MOD_CONTROL != 0 {
        vks.push(0xA2); // LCONTROL
    }
    if modifiers & MOD_SHIFT != 0 {
        vks.push(0xA0); // LSHIFT
    }
    if modifiers & MOD_ALT != 0 {
        vks.push(0xA4); // LMENU
    }
    if modifiers & MOD_WIN != 0 {
        vks.push(0x5B); // LWIN
    }
    vks
}

/// VK → 扫描码 + 扩展位。扫描码没有线性规律（字母区、媒体键都是散布值），
/// 静态表已经被实测打脸过一轮，直接问系统要：MAPVK_VK_TO_VSC_EX 返回
/// 低 8 位扫描码，扩展键带 0xE000 前缀。
/// 注意 Windows 的已知缺陷：方向键与导航簇（Insert/Delete/Home/End/PgUp/PgDn）
/// 是扩展键但返回值不带 E0 前缀，这批必须用硬编码集合兜底——不带
/// KEYEVENTF_EXTENDEDKEY 时 0x4B 会被解释成小键盘 4 而非左方向键。
fn vk_to_scan(vk: u16) -> Option<(u16, bool)> {
    let raw = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC_EX) };
    if raw == 0 {
        return None;
    }
    let ext = raw & 0xE000 != 0 || matches!(vk, 0x21..=0x28 | 0x2D | 0x2E | 0x5D | 0x6F);
    Some(((raw & 0xFF) as u16, ext))
}

fn key_input(scan: u16, extended: bool, up: bool) -> INPUT {
    let mut flags = KEYEVENTF_SCANCODE;
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send(inputs: Vec<INPUT>) -> Result<()> {
    if inputs.is_empty() {
        return Ok(());
    }
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent != inputs.len() as u32 {
        return Err(crate::PlatformError::Message(format!(
            "SendInput 仅注入 {sent}/{} 个事件",
            inputs.len()
        )));
    }
    Ok(())
}

fn chord_events(chord: KeyChord, up: bool) -> Result<Vec<INPUT>> {
    let mut events = Vec::new();
    let mods = modifier_vks(chord.modifiers);
    let main = vk_to_scan(chord.vk)
        .map(|(scan, ext)| key_input(scan, ext, up))
        .ok_or_else(|| crate::PlatformError::Message(format!("不支持的键码 0x{:02X}", chord.vk)))?;
    if !up {
        for &m in &mods {
            if let Some((scan, ext)) = vk_to_scan(m) {
                events.push(key_input(scan, ext, false));
            }
        }
        events.push(main);
    } else {
        events.push(main);
        for &m in mods.iter().rev() {
            if let Some((scan, ext)) = vk_to_scan(m) {
                events.push(key_input(scan, ext, true));
            }
        }
    }
    Ok(events)
}

/// 敲击组合键（按下 + 释放），带小间隔提升兼容性。
pub fn tap(chord: KeyChord) -> Result<()> {
    let down = chord_events(chord, false)?;
    let up = chord_events(chord, true)?;
    send(down)?;
    std::thread::sleep(std::time::Duration::from_millis(30));
    send(up)
}

/// 按住组合键（hold 模式开始）。
pub fn press(chord: KeyChord) -> Result<()> {
    send(chord_events(chord, false)?)
}

/// 释放组合键（hold 模式结束）。
pub fn release(chord: KeyChord) -> Result<()> {
    send(chord_events(chord, true)?)
}

/// 媒体键 / 音量键。
pub fn media(code: MediaKeyCode) -> Result<()> {
    let vk = match code {
        MediaKeyCode::PlayPause => 0xB3,
        MediaKeyCode::Stop => 0xB2,
        MediaKeyCode::Next => 0xB0,
        MediaKeyCode::Previous => 0xB1,
        MediaKeyCode::Mute => 0xAD,
    };
    let Some((scan, ext)) = vk_to_scan(vk) else {
        return Err(crate::PlatformError::Message("媒体键扫描码缺失".into()));
    };
    send(vec![key_input(scan, ext, false)])?;
    std::thread::sleep(std::time::Duration::from_millis(20));
    send(vec![key_input(scan, ext, true)])
}

/// 音量增减（VK_VOLUME_UP/DOWN）。
pub fn volume(down: bool) -> Result<()> {
    let vk = if down { 0xAF } else { 0xAE };
    let Some((scan, ext)) = vk_to_scan(vk) else {
        return Err(crate::PlatformError::Message("音量键扫描码缺失".into()));
    };
    send(vec![key_input(scan, ext, false)])?;
    std::thread::sleep(std::time::Duration::from_millis(20));
    send(vec![key_input(scan, ext, true)])
}

/// 静音。
pub fn volume_mute() -> Result<()> {
    media(MediaKeyCode::Mute)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_codes_cover_action_keys() {
        // 动作表会用到的键全部可解析。
        for vk in [
            0x0D, 0x1B, 0x08, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x2D, 0x2E,
            0x41, 0x43, 0x46, 0x48, 0x4C, 0x4E, 0x50, 0x53, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x5B,
            0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xB0, 0xB1, 0xB2, 0xB3, 0xAD, 0xAE,
            0xAF,
        ] {
            assert!(vk_to_scan(vk).is_some(), "vk 0x{vk:02X} missing scan code");
        }
    }

    #[test]
    fn scan_codes_match_os_ground_truth() {
        // 真值取自 MapVirtualKeyW(MAPVK_VK_TO_VSC_EX) 实测——扫描码没有
        // 线性规律，回归测试必须断言值，不能只断言"能解析"。
        let cases: &[(u16, u16, bool)] = &[
            (0x41, 0x1E, false), // A
            (0x57, 0x11, false), // W
            (0x5A, 0x2C, false), // Z
            (0x30, 0x0B, false), // 0（数字行）
            (0x0D, 0x1C, false), // Enter
            (0x20, 0x39, false), // Space
            (0x70, 0x3B, false), // F1
            (0x74, 0x3F, false), // F5
            (0xAD, 0x20, true),  // VolumeMute
            (0xAE, 0x2E, true),  // VolumeDown
            (0xAF, 0x30, true),  // VolumeUp
            (0xB0, 0x19, true),  // MediaNext
            (0xB1, 0x10, true),  // MediaPrev
            (0xB2, 0x24, true),  // MediaStop
            (0xB3, 0x22, true),  // MediaPlayPause
            (0xA6, 0x6A, true),  // BrowserBack
            (0xA8, 0x67, true),  // BrowserRefresh
            (0xAA, 0x65, true),  // BrowserSearch
            (0xAC, 0x32, true),  // BrowserHome
            (0xB4, 0x6C, true),  // LaunchMail
            (0xB6, 0x6B, true),  // LaunchApp1
            (0x25, 0x4B, true),  // Left
            (0x24, 0x47, true),  // Home
            (0x5B, 0x5B, true),  // LWin
            (0xA2, 0x1D, false), // LControl
            (0xA0, 0x2A, false), // LShift
            (0x60, 0x52, false), // Numpad0
        ];
        for &(vk, scan, ext) in cases {
            assert_eq!(
                vk_to_scan(vk),
                Some((scan, ext)),
                "vk 0x{vk:02X} scan code mismatch"
            );
        }
    }

    #[test]
    fn right_modifiers_use_scan_code_distinction() {
        // 右 Shift 靠扫描码 0x36 区分（非扩展）。
        let (scan, ext) = vk_to_scan(0xA1).unwrap();
        assert_eq!(scan, 0x36);
        assert!(!ext);
        // 右 Ctrl / 右 Alt 是扩展键（扫描码与左侧相同 + EXTENDED）。
        let (scan, ext) = vk_to_scan(0xA3).unwrap();
        assert_eq!(scan, 0x1D);
        assert!(ext);
        let (_, ext) = vk_to_scan(0xA5).unwrap();
        assert!(ext);
    }

    #[test]
    fn navigation_keys_are_extended() {
        for vk in [0x25, 0x26, 0x27, 0x28] {
            let (_, ext) = vk_to_scan(vk).unwrap();
            assert!(ext, "vk 0x{vk:02X} must be extended");
        }
        let (_, ext) = vk_to_scan(0x41).unwrap(); // A
        assert!(!ext);
    }

    #[test]
    fn modifier_order_press_then_release_reverse() {
        let chord = KeyChord::new(0x56, MOD_CONTROL | MOD_SHIFT);
        let down = chord_events(chord, false).unwrap();
        let up = chord_events(chord, true).unwrap();
        assert_eq!(down.len(), 3);
        assert_eq!(up.len(), 3);
    }
}
