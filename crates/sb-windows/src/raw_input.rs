//! 遥控器 HID 捕获：Raw Input（WM_INPUT）线程。
//!
//! 注册全部键盘页 + 消费者页设备，按设备路径过滤 Xiaomi VID 0x2717：
//! - 键盘页报文：F5（usage 0x3E）沿 = 语音键
//! - 消费者页报文：usage 数组（report ID 1）→ 按键集合 → 沿事件
//! 我们的 SendInput 注入不会进入本线程的回调（Raw Input 不过滤 injected，
//! 但键盘注入带我们的 extra info 标记，按 LLKHF_INJECTED 区分由 key_gate 处理；
//! Raw Input 侧按设备句柄过滤，物理遥控器才产生事件）。

use std::sync::mpsc::Sender;
use std::thread::JoinHandle;

use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::{
    GetRawInputData, GetRawInputDeviceInfoW, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE,
    RAWINPUTDEVICE_FLAGS, RAWINPUTHEADER, RegisterRawInputDevices, RIDEV_INPUTSINK,
    RIDI_DEVICENAME, RID_INPUT, RIM_TYPEKEYBOARD,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW,
    TranslateMessage, MSG, WINDOW_STYLE, WNDCLASSW,
};

use sb_core::buttons::{parse_usage_report, VoiceKeyHid};

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
            if let Some(event) = PENDING_EVENT.take() {
                let _ = sender.send(event);
            }
        }
    }
}

thread_local! {
    // 窗口过程回调里直接 send 有重入风险：借 thread-local 暂存，
    // 消息循环取出后转发。
    static PENDING_EVENT: std::cell::RefCell<Option<HidEvent>> = const { std::cell::RefCell::new(None) };
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == windows::Win32::UI::WindowsAndMessaging::WM_INPUT {
        let handled = handle_raw_input(wparam, lparam);
        if handled {
            return LRESULT(0);
        }
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

fn register_raw_input(hwnd: HWND) -> bool {
    // usage page 1（键盘）+ usage page 12（消费者控制）。
    let devices = [
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

fn handle_raw_input(wparam: WPARAM, lparam: LPARAM) -> bool {
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
            return false;
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
            return false;
        }
        let raw = &*(buffer.as_ptr() as *const RAWINPUT);
        let device = raw.header.hDevice;
        // 设备 0 = 泛系统事件；遥控器报文一定带设备句柄。
        if device.is_invalid() || device.0.is_null() {
            return false;
        }
        if !is_xiaomi_device(device) {
            return false;
        }
        let event = parse_raw_input(raw, wparam);
        if let Some(event) = event {
            PENDING_EVENT.with(|slot| *slot.borrow_mut() = Some(event));
            return true;
        }
        false
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

/// 解析 RAWINPUT：键盘页找 F5 usage；消费者页解析 usage 数组报文。
fn parse_raw_input(raw: &RAWINPUT, wparam: WPARAM) -> Option<HidEvent> {
    let front = (wparam.0 & 0xFF) == 0; // RIM_INPUT
    if !front && (wparam.0 & 0xFF) != 1 {
        return None;
    }
    unsafe {
        if raw.header.dwType == RIM_TYPEKEYBOARD.0 {
            let kb = raw.data.keyboard;
            // WM_KEYDOWN=0x100 / WM_SYSKEYDOWN=0x104 为按下。
            let pressed = matches!(kb.Message, 0x100 | 0x104);
            // F5 虚拟键 0x74；真实键盘同样触发——由设备过滤保证只处理遥控器。
            if kb.VKey == 0x74 {
                return Some(HidEvent::VoiceKey { pressed });
            }
            return Some(HidEvent::Activity);
        }
        // HID 报文（消费者页）：data.hid.bRawData。
        let hid = raw.data.hid;
        let raw_data = std::slice::from_raw_parts(
            hid.bRawData.as_ptr(),
            hid.dwSizeHid as usize * hid.dwCount as usize,
        );
        let usages = parse_usage_report(raw_data)?;
        Some(HidEvent::UsageSet(usages))
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
