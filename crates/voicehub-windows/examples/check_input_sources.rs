//! List current Raw Input sources and the selected remote's input filter decision.
//! Usage: cargo run -p sb-windows --example check_input_sources -- <pairedDeviceId>

use voicehub_windows::raw_input::is_selected_remote;
use windows::Win32::UI::Input::{
    GetRawInputDeviceInfoW, GetRawInputDeviceList, RAWINPUTDEVICELIST, RIDI_DEVICENAME,
};

fn main() -> windows::core::Result<()> {
    let selected = std::env::args().nth(1).expect("pairedDeviceId argument required");
    unsafe {
        let mut count = 0;
        let size = std::mem::size_of::<RAWINPUTDEVICELIST>() as u32;
        if GetRawInputDeviceList(None, &mut count, size) == u32::MAX {
            return Err(windows::core::Error::from_thread());
        }
        let mut list = vec![RAWINPUTDEVICELIST::default(); count as usize];
        let got = GetRawInputDeviceList(Some(list.as_mut_ptr()), &mut count, size);
        if got == u32::MAX {
            return Err(windows::core::Error::from_thread());
        }
        let mut accepted = 0;
        for item in list.iter().take(got as usize) {
            let mut len = 0;
            GetRawInputDeviceInfoW(Some(item.hDevice), RIDI_DEVICENAME, None, &mut len);
            if len == 0 {
                continue;
            }
            let mut name = vec![0u16; len as usize];
            if GetRawInputDeviceInfoW(
                Some(item.hDevice), RIDI_DEVICENAME, Some(name.as_mut_ptr().cast()), &mut len,
            ) == u32::MAX {
                return Err(windows::core::Error::from_thread());
            }
            let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
            let path = String::from_utf16_lossy(&name[..end]);
            let allow = is_selected_remote(&path, Some(&selected));
            accepted += usize::from(allow);
            println!("{} type={} {}", if allow { "ACCEPT" } else { "IGNORE" }, item.dwType.0, path);
        }
        println!("Accepted {accepted} of {got} input sources");
    }
    Ok(())
}
