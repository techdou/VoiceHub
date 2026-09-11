//! 遥控器 HID 捕获：Raw Input（WM_INPUT）线程。
//!
//! 注册键盘页 + 消费者页 + 鼠标页设备，只接收选中遥控器的蓝牙 HID：
//! - 键盘页报文：F5（usage 0x3E）沿 = 语音键
//! - 消费者页报文：usage 数组（report ID 1/2）→ 按键集合 → 沿事件
//! - 选中遥控器若提供鼠标集合：左键 → Ok、滚轮 → Up/Down，忽略位移。
//! USB VID_2717/PID_5070 是本机鼠标，不能当作遥控器型号或输入来源。
//! 输入保留设备路径，宿主按当前选择核对蓝牙地址后才计数和执行映射。


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

use voicehub_core::buttons::{parse_usage_report, RemoteButton, VoiceKeyHid};

/// 从 Raw Input 线程发往宿主的事件。
#[derive(Debug, Clone, PartialEq)]
pub enum HidEvent {
    /// 语音键（键盘页 F5）按下 / 释放。
    VoiceKey { pressed: bool },
    /// 消费者页 usage 数组报文（当前按下集合，沿差分由宿主 UsageTracker 做）。
    UsageSet(Vec<u16>),
    /// 触摸板滚轮 tick：语义上已是完成的单击（连续滚动不该被手势识别器
    /// 误判成双击），宿主直接按单击派发，不经手势状态机。
    WheelClick { button: RemoteButton },
    /// 任意遥控器活动（用于诊断/保活统计）。
    Activity,
}

#[derive(Debug)]
pub struct HidInput {
    pub device_path: String,
    pub events: Vec<HidEvent>,
}

impl HidInput {
    pub fn into_events_for(self, selected_id: Option<&str>) -> Vec<HidEvent> {
        if is_selected_remote(&self.device_path, selected_id) {
            self.events
        } else {
            Vec::new()
        }
    }
}

const WM_APP_HID: u32 = 0x8000; // 未用（占位），窗口过程按 WM_INPUT 处理

/// 启动 HID 捕获线程。返回停机句柄。
pub fn spawn_hid_monitor(sender: Sender<HidInput>) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("vh-raw-input".into())
        .spawn(move || run_monitor(sender))
        .map_err(|e| std::io::Error::other(e.to_string()))
}

fn run_monitor(sender: Sender<HidInput>) {
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
    // 消息循环取出后转发。队列而非单槽：一条鼠标报文可能产生多个事件。
    static PENDING_EVENTS: std::cell::RefCell<Vec<HidInput>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_INPUT {
        if let Some(input) = handle_raw_input(wparam, lparam) {
            PENDING_EVENTS.with(|queue| queue.borrow_mut().push(input));
            return LRESULT(0);
        }
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

fn register_raw_input(hwnd: HWND) -> bool {
    // usage page 1（键盘+鼠标）+ usage page 12（消费者控制）。
    // 鼠标集合也必须通过蓝牙来源和选中设备地址校验。
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

/// 触摸板鼠标标志 → 事件序列（纯函数，单测覆盖）。
///
/// 仅用于通过来源校验的遥控器鼠标集合。
/// 左键走 usage 合成（Ok 的按住/释放交给手势识别，支持长按/双击）；
/// 滚轮直接发 WheelClick——tick 本身就是已完成的单击，若走手势状态机，
/// 连续滚动会被 300ms 双击窗口误判成 DoubleClick 而吞掉。
pub fn touchpad_events(button_flags: u32, wheel_delta: i16) -> Vec<HidEvent> {
    if button_flags & RI_MOUSE_LEFT_BUTTON_DOWN != 0 {
        return vec![HidEvent::UsageSet(vec![RemoteButton::Ok.hid_usage()])];
    }
    if button_flags & RI_MOUSE_LEFT_BUTTON_UP != 0 {
        return vec![HidEvent::UsageSet(Vec::new())];
    }
    if button_flags & RI_MOUSE_WHEEL != 0 {
        // 边缘上滑（delta>0）= 向上导航，下滑 = 向下。
        let button = if wheel_delta >= 0 { RemoteButton::Up } else { RemoteButton::Down };
        return vec![HidEvent::WheelClick { button }];
    }
    // 纯位移与其他按钮：不合成。
    Vec::new()
}

fn handle_raw_input(wparam: WPARAM, lparam: LPARAM) -> Option<HidInput> {
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
        let device_path = device_path(device)?;
        // Reject unrelated devices before parsing their reports.
        bluetooth_hid_address(&device_path)?;
        let events = parse_raw_input(raw, wparam);
        if events.is_empty() {
            None
        } else {
            Some(HidInput { device_path, events })
        }
    }
}

fn device_path(device: HANDLE) -> Option<String> {
    unsafe {
        let mut size: u32 = 0;
        let _ = GetRawInputDeviceInfoW(Some(device), RIDI_DEVICENAME, None, &mut size);
        if size == 0 {
            return None;
        }
        let mut name_buf = vec![0u16; size as usize];
        if GetRawInputDeviceInfoW(
            Some(device),
            RIDI_DEVICENAME,
            Some(name_buf.as_mut_ptr() as *mut core::ffi::c_void),
            &mut size,
        ) == u32::MAX
        {
            return None;
        }
        let end = name_buf.iter().position(|&c| c == 0).unwrap_or(name_buf.len());
        Some(String::from_utf16_lossy(&name_buf[..end]))
    }
}

fn bluetooth_hid_address(device_path: &str) -> Option<u64> {
    let lower = device_path.to_ascii_lowercase();
    let hardware = lower.strip_prefix(r"\\?\hid#")?.split('#').next()?;
    let fields = hardware.strip_prefix("{00001812-0000-1000-8000-00805f9b34fb}_dev_")?;
    let mut fields = fields.split('_');
    if fields.next()? != format!("vid&01{:04x}", VoiceKeyHid::VENDOR_ID) {
        return None;
    }
    fields.next()?.strip_prefix("pid&")?;
    fields.next()?.strip_prefix("rev&")?;
    let address = fields.next()?;
    if fields.next().is_some() || address.len() != 12 || !address.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(address, 16).ok()
}

pub fn is_selected_remote(device_path: &str, selected_id: Option<&str>) -> bool {
    let Some(address) = bluetooth_hid_address(device_path) else { return false };
    let Some(selected) = selected_id.and_then(|id| id.strip_prefix("BluetoothLE#BluetoothLE")) else {
        return false;
    };
    let Some((_, remote)) = selected.rsplit_once('-') else { return false };
    let octets: Vec<_> = remote.split(':').collect();
    if octets.len() != 6 || octets.iter().any(|part| part.len() != 2 || !part.bytes().all(|c| c.is_ascii_hexdigit())) {
        return false;
    }
    u64::from_str_radix(&octets.concat(), 16).ok() == Some(address)
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
            // 来源已通过蓝牙 HID 校验，宿主还需核对选中设备。
            let mouse = raw.data.mouse;
            let button_flags = mouse.Anonymous.Anonymous.usButtonFlags as u32;
            let wheel_delta = mouse.Anonymous.Anonymous.usButtonData as i16;
            return touchpad_events(button_flags, wheel_delta);
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
#[derive(Default)]
pub struct UsageTracker {
    previous: Vec<u16>,
}


impl UsageTracker {
    /// usage 集合 → 按键沿（按时间顺序：先释放后按下）。
    pub fn update(&mut self, usages: &[u16]) -> Vec<voicehub_core::buttons::ButtonEdge> {
        let edges = voicehub_core::buttons::diff_usage_sets(&self.previous, usages);
        self.previous = usages.to_vec();
        edges
    }
}

#[allow(dead_code)]
const _: u32 = WM_APP_HID;

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic addresses preserve the Windows path format without exposing device IDs.
    const SELECTED_REMOTE: &str = "BluetoothLE#BluetoothLE00:11:22:33:44:55-aa:bb:cc:dd:ee:ff";
    const REMOTE_HID: &str = r"\\?\HID#{00001812-0000-1000-8000-00805F9B34FB}_DEV_VID&012717_PID&32B8_REV&00A4_AABBCCDDEEFF#C&00000000&0&0000#{GUID}";
    const USB_MOUSE: &str = r"\\?\HID#VID_2717&PID_5070&MI_00&COL01#9&00000000&0&0000#{GUID}";

    #[test]
    fn selected_remote_rejects_xiaomi_usb_mouse() {
        assert!(!is_selected_remote(USB_MOUSE, Some(SELECTED_REMOTE)));
    }

    #[test]
    fn selected_remote_accepts_actual_bluetooth_hid_path() {
        assert!(is_selected_remote(REMOTE_HID, Some(SELECTED_REMOTE)));
        assert!(is_selected_remote(&REMOTE_HID.to_lowercase(), Some(SELECTED_REMOTE)));
    }

    #[test]
    fn selected_remote_rejects_other_devices_and_missing_selection() {
        let other = REMOTE_HID.replace("AABBCCDDEEFF", "AABBCCDDEE00");
        assert!(!is_selected_remote(&other, Some(SELECTED_REMOTE)));
        assert!(!is_selected_remote(REMOTE_HID, None));
        assert!(!is_selected_remote(REMOTE_HID, Some("")));
        assert!(!is_selected_remote(REMOTE_HID, Some("invalid")));
        assert!(!is_selected_remote(&REMOTE_HID.replace("012717", "01046D"), Some(SELECTED_REMOTE)));
    }

    #[test]
    fn selected_remote_drops_mouse_events_before_statistics_and_mapping() {
        for flags in [RI_MOUSE_LEFT_BUTTON_DOWN, RI_MOUSE_LEFT_BUTTON_UP, RI_MOUSE_WHEEL] {
            let input = HidInput {
                device_path: USB_MOUSE.into(),
                events: touchpad_events(flags, 120),
            };
            assert!(input.into_events_for(Some(SELECTED_REMOTE)).is_empty());
        }
    }

    #[test]
    fn selected_remote_preserves_button_and_voice_events() {
        let events = vec![
            HidEvent::UsageSet(vec![RemoteButton::Ok.hid_usage()]),
            HidEvent::UsageSet(Vec::new()),
            HidEvent::VoiceKey { pressed: true },
            HidEvent::VoiceKey { pressed: false },
        ];
        let input = HidInput { device_path: REMOTE_HID.into(), events: events.clone() };
        assert_eq!(input.into_events_for(Some(SELECTED_REMOTE)), events);
    }

    #[test]
    fn selected_remote_rechecks_queued_input_after_selection_changes() {
        let input = HidInput {
            device_path: REMOTE_HID.into(),
            events: vec![HidEvent::UsageSet(vec![RemoteButton::Ok.hid_usage()])],
        };
        let next_remote = SELECTED_REMOTE.replace("ee:ff", "ee:00");
        assert!(input.into_events_for(Some(&next_remote)).is_empty());
    }

    #[test]
    fn selected_remote_rejects_partial_and_malformed_addresses() {
        for remote in ["aa:bb:cc:dd:ee", "aa:bb:cc:dd:ee:zz", "aa:bb:cc:dd:ee:fff"] {
            let selected = format!("BluetoothLE#BluetoothLE00:11:22:33:44:55-{remote}");
            assert!(!is_selected_remote(REMOTE_HID, Some(&selected)));
        }
        for address in ["AABBCCDDEEF", "AABBCCDDEEFFF", "AABBCCDDEEFZ"] {
            let path = REMOTE_HID.replace("AABBCCDDEEFF", address);
            assert!(!is_selected_remote(&path, Some(SELECTED_REMOTE)));
        }
    }

    #[test]
    fn touchpad_tap_maps_to_ok_press_and_release() {
        // 点击触摸板 = 左键 down/up → Ok usage 的按下/空集释放。
        let down = touchpad_events(RI_MOUSE_LEFT_BUTTON_DOWN, 0);
        assert_eq!(
            down,
            vec![HidEvent::UsageSet(vec![RemoteButton::Ok.hid_usage()])]
        );
        let up = touchpad_events(RI_MOUSE_LEFT_BUTTON_UP, 0);
        assert_eq!(up, vec![HidEvent::UsageSet(Vec::new())]);
    }

    #[test]
    fn touchpad_wheel_is_direct_single_click_event() {
        // 上滑（delta>0）→ Up；下滑 → Down。WheelClick 不走手势状态机，
        // 连续滚动不会被 300ms 双击窗口吞掉。
        let up = touchpad_events(RI_MOUSE_WHEEL, 120);
        assert_eq!(up, vec![HidEvent::WheelClick { button: RemoteButton::Up }]);
        let down = touchpad_events(RI_MOUSE_WHEEL, -120);
        assert_eq!(down, vec![HidEvent::WheelClick { button: RemoteButton::Down }]);
    }

    #[test]
    fn pure_movement_and_other_buttons_produce_nothing() {
        assert!(touchpad_events(0, 0).is_empty());
        // 右键 / 中键不映射到遥控器按键。
        assert!(touchpad_events(0x0004, 0).is_empty());
        assert!(touchpad_events(0x0010, 0).is_empty());
    }

    #[test]
    fn tap_feeds_gesture_chain_as_ok_edges() {
        // 左键合成报文过 UsageTracker → Ok 按下+释放沿（手势识别的上游）。
        let mut tracker = UsageTracker::default();
        let down = touchpad_events(RI_MOUSE_LEFT_BUTTON_DOWN, 0);
        let up = touchpad_events(RI_MOUSE_LEFT_BUTTON_UP, 0);
        let mut edges = Vec::new();
        for event in down.iter().chain(up.iter()) {
            if let HidEvent::UsageSet(usages) = event {
                edges.extend(tracker.update(usages));
            }
        }
        assert_eq!(
            edges,
            vec![
                voicehub_core::buttons::ButtonEdge { button: RemoteButton::Ok, pressed: true },
                voicehub_core::buttons::ButtonEdge { button: RemoteButton::Ok, pressed: false },
            ]
        );
    }
}
