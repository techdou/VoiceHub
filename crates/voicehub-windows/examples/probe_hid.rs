//! 探针：枚举 Xiaomi HID 设备 + dump 遥控器 Raw Input 报文（真机诊断）。
//!
//! 用途：定位"按键设置不生效"的断点——statistics.json 显示 buttonPressCount=0，
//! 说明消费页报文从未被解析成按键沿。本探针打印：
//! 1. 系统里所有 Xiaomi (VID 0x2717) 原始输入设备及类型；
//! 2. 每次 WM_INPUT 的原始报文（hex）、当前解析器 parse_usage_report 的结果、
//!    usage→RemoteButton 映射结果。
//! 运行：cargo run -p sb-windows --example probe_hid，然后按遥控器按键。

use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::{
    GetRawInputData, GetRawInputDeviceInfoW, GetRawInputDeviceList, HRAWINPUT, RAWINPUT,
    RAWINPUTDEVICE, RAWINPUTDEVICE_FLAGS, RAWINPUTDEVICELIST, RIDI_DEVICENAME,
    RIDI_PREPARSEDDATA, RID_INPUT, RIDEV_INPUTSINK, RIM_TYPEKEYBOARD,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW,
    TranslateMessage, MSG, WINDOW_STYLE, WM_INPUT, WNDCLASSW,
};

fn dump_preparsed(device: HANDLE) -> Option<u32> {
    unsafe {
        let mut size: u32 = 0;
        let _ = GetRawInputDeviceInfoW(Some(device), RIDI_PREPARSEDDATA, None, &mut size);
        if size == 0 {
            println!("    (no preparsed data)");
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        if GetRawInputDeviceInfoW(
            Some(device),
            RIDI_PREPARSEDDATA,
            Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
            &mut size,
        ) == u32::MAX
        {
            println!("    (preparsed read failed)");
            return None;
        }
        let preparsed = windows::Win32::Devices::HumanInterfaceDevice::PHIDP_PREPARSED_DATA(
            buf.as_ptr() as isize
        );
        use windows::Win32::Devices::HumanInterfaceDevice::{
            HidP_GetButtonCaps, HidP_GetCaps, HidP_Input, HIDP_BUTTON_CAPS, HIDP_CAPS,
        };
        let mut caps = HIDP_CAPS::default();
        if HidP_GetCaps(preparsed, &mut caps).0 != 0x0011_0000 {
            println!("    (HidP_GetCaps failed)");
            return None;
        }
        println!(
            "    TLC usagePage=0x{:04X} usage=0x{:04X} inputReportBytes={} outputReportBytes={}",
            caps.UsagePage, caps.Usage, caps.InputReportByteLength, caps.OutputReportByteLength
        );
        let mut btn_caps = [HIDP_BUTTON_CAPS::default(); 64];
        let mut count = btn_caps.len() as u16;
        let status = HidP_GetButtonCaps(
            HidP_Input,
            btn_caps.as_mut_ptr(),
            &mut count,
            preparsed,
        );
        if status.0 == 0x0011_0000 && count > 0 {
            for bc in &btn_caps[..count as usize] {
                // BitField bit0：0 = usage array（报文是 usage 列表），1 = bitfield（每位一个键）。
                let kind = if bc.BitField & 0x1 == 0 { "ARRAY" } else { "BITFIELD" };
                let usage_desc = if bc.IsRange {
                    let r = &bc.Anonymous.Range;
                    format!("usageMin=0x{:04X}..0x{:04X}", r.UsageMin, r.UsageMax)
                } else {
                    let n = &bc.Anonymous.NotRange;
                    format!("usage=0x{:04X}", n.Usage)
                };
                println!(
                    "    buttonCaps reportID={} page=0x{:04X} {} {} reportCount={} bitField=0x{:02X}",
                    bc.ReportID, bc.UsagePage, kind, usage_desc, bc.ReportCount, bc.BitField
                );
            }
        } else {
            println!("    (no button caps, status=0x{:08X} count={count})", status.0);
        }
        Some(caps.InputReportByteLength as u32)
    }
}

fn device_name(device: HANDLE) -> Option<String> {
    unsafe {
        let mut size: u32 = 0;
        let _ = GetRawInputDeviceInfoW(Some(device), RIDI_DEVICENAME, None, &mut size);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u16; size as usize];
        if GetRawInputDeviceInfoW(
            Some(device),
            RIDI_DEVICENAME,
            Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
            &mut size,
        ) == u32::MAX
        {
            return None;
        }
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..end]))
    }
}

fn short_name(name: &str) -> String {
    // "\\\\?\\hid#vid_2717&pid_32b8#..." → "vid_2717&pid_32b8"
    let trimmed = name.rsplit('\\').next().unwrap_or(name);
    trimmed.to_string()
}

fn is_xiaomi(device: HANDLE) -> Option<String> {
    let name = device_name(device)?;
    if name.to_lowercase().contains("vid_2717") {
        Some(short_name(&name))
    } else {
        None
    }
}

/// 设备路径 → 短标签（Col01/Col02/Col04/MI_01/MI_02）。
fn collection_tag(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower.contains("mi_00&col01") {
        "MI00-Col01(mouse)".into()
    } else if lower.contains("mi_00&col02") {
        "MI00-Col02(consumer)".into()
    } else if lower.contains("mi_00&col04") {
        "MI00-Col04(vendor-ff01)".into()
    } else if lower.contains("mi_01") {
        "MI01(keyboard)".into()
    } else if lower.contains("mi_02") {
        "MI02(vendor-ffef)".into()
    } else {
        short_name(name)
    }
}

fn enumerate_xiaomi_devices() {
    println!("=== Raw Input device list (Xiaomi VID 0x2717) ===");
    unsafe {
        let mut count: u32 = 0;
        let _ = GetRawInputDeviceList(
            None,
            &mut count,
            std::mem::size_of::<RAWINPUTDEVICELIST>() as u32,
        );
        if count == 0 {
            println!("(no raw input devices)");
            return;
        }
        let mut list = vec![RAWINPUTDEVICELIST::default(); count as usize];
        let got = GetRawInputDeviceList(
            Some(list.as_mut_ptr()),
            &mut count,
            std::mem::size_of::<RAWINPUTDEVICELIST>() as u32,
        );
        let mut found = 0;
        for item in list.iter().take(got as usize) {
            let label = match item.dwType.0 {
                0 => "MOUSE",
                1 => "KEYBOARD",
                2 => "HID",
                _ => "OTHER",
            };
            if let Some(name) = is_xiaomi(item.hDevice) {
                println!("  [{label}] {name}");
                if item.dwType.0 == 2 {
                    let report_bytes = dump_preparsed(item.hDevice);
                    let lower = name.to_lowercase();
                    let direct_tag: Option<&'static str> = if lower.contains("col02") {
                        Some("consumer")
                    } else if lower.contains("col04") {
                        Some("vendor-ff01")
                    } else if lower.contains("mi_02") {
                        Some("vendor-ffef")
                    } else {
                        None
                    };
                    if let (Some(tag), Some(bytes)) = (direct_tag, report_bytes) {
                        if bytes > 0 {
                            if let Some(full_path) = device_name(item.hDevice) {
                                spawn_direct_read(tag, full_path, bytes as usize);
                            }
                        }
                    }
                }
                found += 1;
            }
        }
        if found == 0 {
            println!("  (no xiaomi devices found)");
        }
    }
    println!();
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_INPUT {
        if let Some((device_tag, dump)) = handle_raw_input(wparam, lparam) {
            println!("[{device_tag}] {dump}");
        }
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

unsafe fn handle_raw_input(_wparam: WPARAM, lparam: LPARAM) -> Option<(String, String)> {
    let mut size: u32 = 0;
    let header_size = std::mem::size_of::<windows::Win32::UI::Input::RAWINPUTHEADER>() as u32;
    let _ = GetRawInputData(
        HRAWINPUT(lparam.0 as *mut core::ffi::c_void),
        RID_INPUT,
        None,
        &mut size,
        header_size,
    );
    if size == 0 || size > 65536 {
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
    if device.is_invalid() || device.0.is_null() {
        return None;
    }
    // 全设备都打印（对照组：真键盘 vs 遥控器），tag 区分。
    let tag = device_name(device)
        .map(|n| collection_tag(&n))
        .unwrap_or_else(|| "other".into());

    if raw.header.dwType == RIM_TYPEKEYBOARD.0 {
        let kb = raw.data.keyboard;
        let pressed = matches!(kb.Message, 0x100 | 0x104);
        return Some((
            format!("{tag} kb"),
            format!(
                "vkey=0x{:02X} msg=0x{:03X} {} scan=0x{:02X}",
                kb.VKey,
                kb.Message,
                if pressed { "DOWN" } else { "UP  " },
                kb.MakeCode
            ),
        ));
    }

    if raw.header.dwType == windows::Win32::UI::Input::RIM_TYPEMOUSE.0 {
        // 触摸板滑动高频刷屏：只在有按键沿或位移较大时打印。
        let mouse = raw.data.mouse;
        let btns = unsafe { mouse.Anonymous.Anonymous.usButtonFlags };
        if btns == 0 && mouse.lLastX.abs() < 30 && mouse.lLastY.abs() < 30 {
            return None;
        }
        return Some((
            format!("{tag} mouse"),
            format!(
                "lastX={} lastY={} flags=0x{:04X} btnFlags=0x{:04X} rawButtons=0x{:08X}",
                mouse.lLastX, mouse.lLastY, mouse.usFlags.0, btns, mouse.ulRawButtons
            ),
        ));
    }

    // dwType == 2 (HID)：消费页等通用 HID 报文。
    let hid = raw.data.hid;
    let len = hid.dwSizeHid as usize * hid.dwCount as usize;
    let data = std::slice::from_raw_parts(hid.bRawData.as_ptr(), len);
    let hex: Vec<String> = data.iter().map(|b| format!("{b:02X}")).collect();
    let parsed = voicehub_core::buttons::parse_usage_report(data);
    let detail = match parsed {
        None => "parse_usage_report -> None (REJECTED)".to_string(),
        Some(usages) => {
            let mapped: Vec<String> = usages
                .iter()
                .map(|&u| {
                    match voicehub_core::buttons::RemoteButton::from_hid_usage(u) {
                        Some(b) => format!("0x{u:04X}=>{b:?}"),
                        None => format!("0x{u:04X}=>UNMAPPED"),
                    }
                })
                .collect();
            if usages.is_empty() {
                "(empty = all released)".to_string()
            } else {
                mapped.join(" ")
            }
        }
    };
    Some((
        format!("{tag} hid"),
        format!(
            "sizeHid={} count={} bytes=[{}] {}",
            hid.dwSizeHid,
            hid.dwCount,
            hex.join(" "),
            detail
        ),
    ))
}

/// 直接 ReadFile 读 HID 集合（绕开 Raw Input——usage=0 的 vendor TLC 注册不了）。
fn spawn_direct_read(tag: &'static str, name: String, report_bytes: usize) {
    std::thread::spawn(move || unsafe {
        use windows::Win32::Foundation::GENERIC_READ;
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, ReadFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
            OPEN_EXISTING,
        };
        let path: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let handle = CreateFileW(
            windows::core::PCWSTR(path.as_ptr()),
            GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        );
        let Ok(handle) = handle else {
            println!("[direct:{tag}] CreateFile failed: {}", handle.err().map(|e| e.to_string()).unwrap_or_default());
            return;
        };
        let cap = report_bytes.max(8);
        let mut buf = vec![0u8; cap];
        println!("[direct:{tag}] reading (reportBytes={report_bytes})...");
        loop {
            let mut read = 0u32;
            // ReadFile 在 windows crate 0.62 返回 Result<()>。
            if ReadFile(handle, Some(&mut buf), Some(&mut read), None).is_err() {
                println!("[direct:{tag}] ReadFile failed");
                return;
            }
            if read == 0 {
                continue;
            }
            let hex: Vec<String> = buf[..read as usize].iter().map(|b| format!("{b:02X}")).collect();
            println!("[direct:{tag}] {read}B [{}]", hex.join(" "));
        }
    });
}

fn main() {
    enumerate_xiaomi_devices();
    println!("listening for WM_INPUT (keyboard + consumer page)... press Ctrl+C to stop");
    println!("--- now press remote buttons ---");

    unsafe {
        let class_name: Vec<u16> = "ProbeHidWindow".encode_utf16().chain(Some(0)).collect();
        let class_ptr = windows::core::PCWSTR(class_name.as_ptr());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            lpszClassName: class_ptr,
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            eprintln!("RegisterClassW failed");
            std::process::exit(1);
        }
        let window_name: Vec<u16> = "Probe HID".encode_utf16().chain(Some(0)).collect();
        let hwnd = CreateWindowExW(
            Default::default(),
            class_ptr,
            windows::core::PCWSTR(window_name.as_ptr()),
            WINDOW_STYLE(0),
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
            eprintln!("CreateWindowExW failed");
            std::process::exit(1);
        };
        let devices = [
            RAWINPUTDEVICE {
                usUsagePage: 0x01,
                usUsage: 0x02, // Mouse（遥控器 MI_00 Col01 是鼠标集合）
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
        if !RegisterRawInputDevicesCheck(&devices) {
            eprintln!("RegisterRawInputDevices failed");
            std::process::exit(1);
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn RegisterRawInputDevicesCheck(devices: &[RAWINPUTDEVICE]) -> bool {
    unsafe {
        windows::Win32::UI::Input::RegisterRawInputDevices(
            devices,
            std::mem::size_of::<RAWINPUTDEVICE>() as u32,
        )
        .is_ok()
    }
}
