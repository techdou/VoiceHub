//! 注册表检测：VB-CABLE 内核驱动服务（VBAudioVACMME）存在性。
//! 比端点名检测更底层（音频服务未枚举也能判定驱动已装）。

const DRIVER_SERVICE: &str = r"VBAudioVACMME";

pub fn driver_service_present() -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE,
    };
    let mut wide = DRIVER_SERVICE.encode_utf16().collect::<Vec<_>>();
    wide.push(0);
    let mut key = Default::default();
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(wide.as_ptr()),
            None,
            KEY_QUERY_VALUE,
            &mut key,
        )
    };
    if opened.is_ok() {
        unsafe {
            let _ = RegCloseKey(key);
        }
        true
    } else {
        false
    }
}
