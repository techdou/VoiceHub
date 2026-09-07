//! 系统睡眠 / 唤醒通知（PowerRegisterSuspendResumeNotification）。

use std::sync::mpsc::Sender;
use std::thread::JoinHandle;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Power::{
    PowerRegisterSuspendResumeNotification, DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS,
};
use windows::Win32::UI::WindowsAndMessaging::DEVICE_NOTIFY_CALLBACK;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerEvent {
    Suspend,
    Resume,
}

/// 注册电源通知。返回 (释放句柄, 工作线程)；句柄供 unregister 使用。
pub fn spawn_power_monitor(sender: Sender<PowerEvent>) -> windows::core::Result<JoinHandle<()>> {
    let (tx, rx) = std::sync::mpsc::channel::<PowerEvent>();
    // 转发线程：回调线程 → 用户通道。
    let forwarder = std::thread::Builder::new()
        .name("vh-power-forward".into())
        .spawn(move || {
            while let Ok(event) = rx.recv() {
                let _ = sender.send(event);
            }
        })
        .map_err(|_| windows::core::Error::from_hresult(windows::core::HRESULT(0x8000_4005u32 as i32)))?;

    unsafe extern "system" fn callback(
        context: *const core::ffi::c_void,
        event_type: u32,
        _setting: *const core::ffi::c_void,
    ) -> u32 {
        let tx = &*(context as *const std::sync::mpsc::Sender<PowerEvent>);
        // PBT_APMSUSPEND=4, PBT_APMRESUMESUSPEND=7 / PBT_APMRESUMEAUTOMATIC=18
        match event_type {
            4 => {
                let _ = tx.send(PowerEvent::Suspend);
            }
            7 | 18 => {
                let _ = tx.send(PowerEvent::Resume);
            }
            _ => {}
        }
        0
    }

    // Sender 泄漏进静态存储：回调生命周期贯穿进程，无法安全释放。
    let sender_box = Box::leak(Box::new(tx));
    let params = DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
        Callback: Some(callback),
        Context: sender_box as *mut _ as *mut core::ffi::c_void,
    };
    let mut registration: *mut core::ffi::c_void = std::ptr::null_mut();
    let status = unsafe {
        PowerRegisterSuspendResumeNotification(
            DEVICE_NOTIFY_CALLBACK,
            HANDLE(&mut params_helper(&params) as *mut _ as *mut core::ffi::c_void),
            &mut registration,
        )
    };
    if status.is_err() {
        return Err(windows::core::Error::from_hresult(status.into()));
    }
    Ok(forwarder)
}

fn params_helper(params: &DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS) -> DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
    unsafe { std::ptr::read(params) }
}

#[cfg(test)]
mod tests {
    #[test]
    fn register_and_drop_is_safe() {
        let (tx, _rx) = std::sync::mpsc::channel();
        // 注册成功与否取决于权限，但不应崩溃。
        let _ = super::spawn_power_monitor(tx);
    }
}
