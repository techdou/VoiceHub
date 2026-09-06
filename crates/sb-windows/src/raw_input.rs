//! 遥控器 HID 捕获：Raw Input（WM_INPUT）线程。
//!
//! 注册键盘页 + 消费者页 + 鼠标页设备，按设备路径过滤 Xiaomi VID 0x2717：
//! - 键盘页报文：F5（usage 0x3E）沿 = 语音键
//! - 消费者页报文：usage 数组（report ID 1/2）→ 按键集合 → 沿事件
//! - 鼠标报文（RC003 PID 0x5070 触摸板遥控器，2026-09 真机实测）：
//!   导航全部走鼠标集合——触摸板点击 = 左键、边缘滑 = 滚轮、滑动 = 位移，
//!   消费者页与键盘页（除 F5）均无报文。翻译：左键 → Ok、滚轮 → Up/Down
//!   瞬时单击；纯位移忽略（光标移动由系统透传）。
//! 我们的 SendInput 注入不会进入本线程的回调（Raw Input 不过滤 injected，
//! 但键盘注入带我们的 extra info 标记，按 LLKHF_INJECTED 区分由 key_gate 处理；
//! Raw Input 侧按设备句柄过滤，物理遥控器才产生事件）。

use std::cell::Cell;
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;

use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::{
    GetRawInputData, GetRawInputDeviceInfoW, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE,
    RAWINPUTDEVICE_FLAGS, RAWINPUTHEADER, RegisterRawInputDevices, RIDEV_INPUTSINK,
    RIDI_DEVICENAME, RID_INPUT, RIM_TYPEKEYBOARD, RIM_TYPEMOUSE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW,
    TranslateMessage, MSG, RI_MOUSE_LEFT_BUTTON_DOWN, RI_MOUSE_LEFT_BUTTON_UP, RI_MOUSE_WHEEL,
    WINDOW_STYLE, WM_INPUT, WNDCLASSW,
};

use sb_core::buttons::{parse_usage_report, RemoteButton, VoiceKeyHid};

/// 从 Raw Input 线程发往宿主的事件。
#[derive(Debug, Clone, PartialEq)]
pub enum HidEvent {
    /// 语音键（键盘页 F5）按下 / 释放。
    VoiceKey { pressed: bool },
    /// 消费者页 usage 数组报文（当前按下集合，沿差分由宿主 UsageTracker 做）。
    UsageSet(Vec<u16>),
    /// 任意遥控器活动（用于诊断/保活统计）。
    Activity,
}

const WM_APP_HID: u32 = 0x8000; // 未用（占位），窗口过程按 WM_INPUT 处理

/// 启动 HID 捕获线程。返回停机句柄。
pub fn spawn_hid_monitor(sender: Sender<HidEvent>) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("sb-raw-input".into())
        .spawn(move || run_monitor(sender))
        .map_err(|e| std::io::Error::other(e.to_string()))
}

fn run_monitor(sender: Sender<HidEvent>) {
    unsafe {
        let class_name_wide: Vec<u16> = "SoundBridgeHidWindow"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let class_name_ptr = windows::core::PCWSTR(class_name_wide.as_ptr());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            lpszClassName: class_name_ptr,
            ..Default::default()
        };
        let atom = RegisterClassW(&wc);
        if atom == 0 {
            log::error!("RegisterClassW failed");
            return;
        }
        let window_name_wide: Vec<u16> = "SoundBridge HID"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let hwnd = CreateWindowExW(
            Default::default(),
            class_name_ptr,
            windows::core::PCWSTR(window_name_wide.as_ptr()),
            WINDOW_STYLE(0), // 不可见窗口（message-only 不支持 Raw Input）
            0,
            0,
            0,
            0,
            None,
            None,
            None,
            None,
        );
        let Ok(hwnd) = hwnd else {
            log::error!("CreateWindowExW failed");
            return;
        };
        if !register_raw_input(hwnd) {
            log::error!("RegisterRawInputDevices failed");
            return;
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
            PENDING_EVENTS.with(|queue| {
                let mut queue = queue.borrow_mut();
                for event in queue.drain(..) {
                    let _ = sender.send(event);
                }
            });
        }
    }
}

thread_local! {
    // 窗口过程回调里直接 send 有重入风险：借 thread-local 暂存，
    // 消息循环取出后转发。队列而非单槽：滚轮一次产生 down+up 两条合成报文。
    static PENDING_EVENTS: std::cell::RefCell<Vec<HidEvent>> =
        const { std::cell::RefCell::new(Vec::new()) };
    // 触摸板左键（=Ok）当前是否按住；滚轮合成瞬时单击时保留其按下状态。
    static OK_HELD: Cell<bool> = const { Cell::new(false) };
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_INPUT {
        if let Some(events) = handle_raw_input(wparam, lparam) {
            PENDING_EVENTS.with(|queue| queue.borrow_mut().extend(events));
            return LRESULT(0);
        }
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

fn register_raw_input(hwnd: HWND) -> bool {
    // usage page 1（键盘+鼠标）+ usage page 12（消费者控制）。
    // 鼠标页：RC003 触摸板遥控器的导航（点击/滚轮/位移）全走鼠标集合。
    let devices = [
        RAWINPUTDEVICE {
            usUsagePage: 0x01,
            usUsage: 0x02, // Mouse
            dwFlags: RAWINPUTDEVICE_FLAGS(RIDEV_INPUTSINK.0),
            hwndTarget: hwnd,
        },
        RAWINPUTDEVICE {
            usUsagePage: 0x01,
            usUsage: 0x06, // Keyboard
            dwFlags: RAWINPUTDEVICE_FLAGS(RIDEV_INPUTSINK.0),
            hwndTarget: hwnd,
        },
        RAWINPUTDEVICE {
            usUsagePage: 0x0C,
            usUsage: 0x01, // Consumer Control
            dwFlags: RAWINPUTDEVICE_FLAGS(RIDEV_INPUTSINK.0),
            hwndTarget: hwnd,
        },
    ];
    unsafe { RegisterRawInputDevices(&devices, std::mem::size_of::<RAWINPUTDEVICE>() as u32) }
        .is_ok()
}

/// 触摸板鼠标标志 → 合成 usage 报文序列（纯函数，单测覆盖）。
///
/// RC003 真机语义（2026-09 实测）：点击=左键、边缘滑=滚轮。
/// 滚轮产生"瞬时单击"两条报文（按下→释放），并把 `ok_held` 的按下状态
/// 带进两条报文，避免长按 Ok 期间滚轮丢失按住状态。
pub fn touchpad_usage_reports(button_flags: u32, wheel_delta: i16, ok_held: bool) -> Vec<Vec<u16>> {
    let ok = RemoteButton::Ok.hid_usage();
    if button_flags & RI_MOUSE_LEFT_BUTTON_DOWN != 0 {
        return vec![vec![ok]];
    }
    if button_flags & RI_MOUSE_LEFT_BUTTON_UP != 0 {
        return vec![Vec::new()];
    }
    if button_flags & RI_MOUSE_WHEEL != 0 {
        let held: Vec<u16> = if ok_held { vec![ok] } else { Vec::new() };
        // 边缘上滑 = 向上导航，下滑 = 向下（触摸板上滑 delta 为正）。
        let nav = if wheel_delta >= 0 {
            RemoteButton::Up.hid_usage()
        } else {
            RemoteButton::Down.hid_usage()
        };
        let mut with_nav = held.clone();
        with_nav.push(nav);
        return vec![with_nav, held];
    }
    // 纯位移与其他按钮：不合成。
    Vec::new()
}

fn handle_raw_input(wparam: WPARAM, lparam: LPARAM) -> Option<Vec<HidEvent>> {
    unsafe {
        let mut size: u32 = 0;
        let header_size = std::mem::size_of::<RAWINPUTHEADER>() as u32;
        let _ = GetRawInputData(
            HRAWINPUT(lparam.0 as *mut core::ffi::c_void),
            RID_INPUT,
            None,
            &mut size,
            header_size,
        );
        if size == 0 || size > 4096 {
            return None;
        }
        let mut buffer = vec![0u8; size as usize];
        if GetRawInputData(
            HRAWINPUT(lparam.0 as *mut core::ffi::c_void),
            RID_INPUT,
            Some(buffer.as_mut_ptr() as *mut core::ffi::c_void),
            &mut size,
            header_size,
        ) == u32::MAX
        {
            return None;
        }
        let raw = &*(buffer.as_ptr() as *const RAWINPUT);
        let device = raw.header.hDevice;
        // 设备 0 = 泛系统事件；遥控器报文一定带设备句柄。
        if device.is_invalid() || device.0.is_null() {
            return None;
        }
        if !is_xiaomi_device(device) {
            return None;
        }
        let events = parse_raw_input(raw, wparam);
        if events.is_empty() {
            None
        } else {
            Some(events)
        }
    }
}

/// 设备句柄 → 设备路径 → 是否 Xiaomi VID。
fn is_xiaomi_device(device: HANDLE) -> bool {
    unsafe {
        let mut size: u32 = 0;
        let _ = GetRawInputDeviceInfoW(Some(device), RIDI_DEVICENAME, None, &mut size);
        if size == 0 {
            return false;
        }
        let mut name_buf = vec![0u16; size as usize];
        if GetRawInputDeviceInfoW(
            Some(device),
            RIDI_DEVICENAME,
            Some(name_buf.as_mut_ptr() as *mut core::ffi::c_void),
            &mut size,
        ) == u32::MAX
        {
            return false;
        }
        let name = String::from_utf16_lossy(&name_buf);
        let lower = name.to_lowercase();
        lower.contains(&format!("vid_{:04x}", VoiceKeyHid::VENDOR_ID))
    }
}

/// 解析 RAWINPUT：键盘页找 F5 usage；鼠标页翻译触摸板导航；
/// 消费者页解析 usage 数组报文。
fn parse_raw_input(raw: &RAWINPUT, wparam: WPARAM) -> Vec<HidEvent> {
    let front = (wparam.0 & 0xFF) == 0; // RIM_INPUT
    if !front && (wparam.0 & 0xFF) != 1 {
        return Vec::new();
    }
    unsafe {
        if raw.header.dwType == RIM_TYPEKEYBOARD.0 {
            let kb = raw.data.keyboard;
            // WM_KEYDOWN=0x100 / WM_SYSKEYDOWN=0x104 为按下。
            let pressed = matches!(kb.Message, 0x100 | 0x104);
            // F5 虚拟键 0x74；真实键盘同样触发——由设备过滤保证只处理遥控器。
            if kb.VKey == 0x74 {
                return vec![HidEvent::VoiceKey { pressed }];
            }
            return vec![HidEvent::Activity];
        }
        if raw.header.dwType == RIM_TYPEMOUSE.0 {
            // RC003 触摸板：点击 = 左键（→Ok），边缘滑 = 滚轮（→Up/Down）。
            let mouse = raw.data.mouse;
            let button_flags = mouse.Anonymous.Anonymous.usButtonFlags as u32;
            let wheel_delta = mouse.Anonymous.Anonymous.usButtonData as i16;
            let reports = touchpad_usage_reports(
                button_flags,
                wheel_delta,
                OK_HELD.with(|held| held.get()),
            );
            if button_flags & RI_MOUSE_LEFT_BUTTON_DOWN != 0 {
                OK_HELD.with(|held| held.set(true));
            }
            if button_flags & RI_MOUSE_LEFT_BUTTON_UP != 0 {
                OK_HELD.with(|held| held.set(false));
            }
            return reports
                .into_iter()
                .map(HidEvent::UsageSet)
                .collect();
        }
        // HID 报文（消费者页）：data.hid.bRawData。
        let hid = raw.data.hid;
        let raw_data = std::slice::from_raw_parts(
            hid.bRawData.as_ptr(),
            hid.dwSizeHid as usize * hid.dwCount as usize,
        );
        let Some(usages) = parse_usage_report(raw_data) else {
            return Vec::new();
        };
        vec![HidEvent::UsageSet(usages)]
    }
}

/// usage 集合流 → 按键沿（宿主侧调用；也可在监控线程内做）。
pub struct UsageTracker {
    previous: Vec<u16>,
}

impl Default for UsageTracker {
    fn default() -> Self {
        Self { previous: Vec::new() }
    }
}

impl UsageTracker {
    /// usage 集合 → 按键沿（按时间顺序：先释放后按下）。
    pub fn update(&mut self, usages: &[u16]) -> Vec<sb_core::buttons::ButtonEdge> {
        let edges = sb_core::buttons::diff_usage_sets(&self.previous, usages);
        self.previous = usages.to_vec();
        edges
    }
}

#[allow(dead_code)]
const _: u32 = WM_APP_HID;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touchpad_tap_maps_to_ok_press_and_release() {
        // 点击触摸板 = 左键 down/up → Ok usage 的按下/空集释放。
        let down = touchpad_usage_reports(RI_MOUSE_LEFT_BUTTON_DOWN, 0, false);
        assert_eq!(down, vec![vec![RemoteButton::Ok.hid_usage()]]);
        let up = touchpad_usage_reports(RI_MOUSE_LEFT_BUTTON_UP, 0, true);
        assert_eq!(up, vec![Vec::<u16>::new()]);
    }

    #[test]
    fn touchpad_wheel_synthesizes_instant_single_click() {
        // 上滑（delta>0）→ Up 瞬时单击；下滑 → Down。
        let up = touchpad_usage_reports(RI_MOUSE_WHEEL, 120, false);
        assert_eq!(
            up,
            vec![
                vec![RemoteButton::Up.hid_usage()],
                Vec::<u16>::new(),
            ]
        );
        let down = touchpad_usage_reports(RI_MOUSE_WHEEL, -120, false);
        assert_eq!(
            down,
            vec![
                vec![RemoteButton::Down.hid_usage()],
                Vec::<u16>::new(),
            ]
        );
    }

    #[test]
    fn wheel_while_ok_held_keeps_ok_in_both_reports() {
        // 长按 Ok 期间滚轮：两条报文都保留 Ok 按住状态，
        // 否则 UsageTracker 会误判 Ok 已释放。
        let events = touchpad_usage_reports(RI_MOUSE_WHEEL, 120, true);
        let ok = RemoteButton::Ok.hid_usage();
        assert_eq!(events, vec![vec![ok, RemoteButton::Up.hid_usage()], vec![ok]]);
    }

    #[test]
    fn pure_movement_and_other_buttons_produce_nothing() {
        assert!(touchpad_usage_reports(0, 0, false).is_empty());
        // 右键 / 中键不属于 RC003 触摸板语义。
        assert!(touchpad_usage_reports(0x0004, 0, false).is_empty());
        assert!(touchpad_usage_reports(0x0010, 0, false).is_empty());
    }

    #[test]
    fn wheel_feeds_gesture_chain_as_single_click() {
        // 端到端（不经 Win32）：滚轮两条报文过 UsageTracker → Up 按下+释放沿。
        let mut tracker = UsageTracker::default();
        let reports = touchpad_usage_reports(RI_MOUSE_WHEEL, 120, false);
        let mut edges = Vec::new();
        for report in &reports {
            edges.extend(tracker.update(report));
        }
        assert!(edges.contains(&sb_core::buttons::ButtonEdge {
            button: RemoteButton::Up,
            pressed: true,
        }));
        assert!(edges.contains(&sb_core::buttons::ButtonEdge {
            button: RemoteButton::Up,
            pressed: false,
        }));
    }
}
