//! 探针：走项目同款 send_input 注入 VK_RMENU（右 Alt），
//! 用 LL 钩子 + GetAsyncKeyState 双视角看系统把它记成了哪个键。
//! 回答“注入的到底是左 Alt 还是右 Alt”。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, SetWindowsHookExW, WH_KEYBOARD_LL,
};
use windows::Win32::UI::WindowsAndMessaging::{KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED};

use voicehub_windows::send_input::{press, release, tap, KeyChord};

static RUNNING: AtomicBool = AtomicBool::new(true);
static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn async_pressed(vk: i32) -> bool {
    (unsafe { GetAsyncKeyState(vk) } as u16) & 0x8000 != 0
}

unsafe extern "system" fn hook_proc(
    code: i32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };
    if code >= 0 && RUNNING.load(Ordering::Relaxed) {
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let kind = match wparam.0 as u32 {
            WM_KEYDOWN | WM_SYSKEYDOWN => "DOWN",
            WM_KEYUP | WM_SYSKEYUP => "UP",
            _ => "OTHER",
        };
        let injected = (kb.flags & LLKHF_INJECTED).0 != 0;
        let extended = (kb.flags & LLKHF_EXTENDED).0 != 0;
        EVENTS.lock().unwrap().push(format!(
            "{kind} vk=0x{:02X} scan=0x{:04X} injected={injected} extended={extended}",
            kb.vkCode, kb.scanCode
        ));
    }
    CallNextHookEx(None, code, wparam, lparam)
}

fn main() {
    // 钩子线程（LL 钩子要求消息泵）。
    std::thread::Builder::new()
        .name("probe-hook".into())
        .spawn(unsafe {
            || {
                let Ok(hook) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0) else {
                    eprintln!("钩子安装失败");
                    return;
                };
                let mut msg = Default::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    DispatchMessageW(&msg);
                }
                let _ = windows::Win32::UI::WindowsAndMessaging::UnhookWindowsHookEx(hook);
            }
        })
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));

    println!("=== 基线（未注入）===");
    println!("  LMENU按下={} RMENU按下={}", async_pressed(0xA4), async_pressed(0xA5));

    println!("=== press(VK_RMENU 扫描码路径) ===");
    press(KeyChord::new(0xA5, 0)).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(80));
    println!(
        "  按住中：LMENU={} RMENU={} MENU(合并)={}",
        async_pressed(0xA4),
        async_pressed(0xA5),
        async_pressed(0x12)
    );
    release(KeyChord::new(0xA5, 0)).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(120));
    println!(
        "  释放后：LMENU={} RMENU={}",
        async_pressed(0xA4),
        async_pressed(0xA5)
    );

    println!("=== tap(VK_RMENU) ===");
    tap(KeyChord::new(0xA5, 0)).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(150));

    println!("=== tap(VK_LMENU 0xA4 对照) ===");
    tap(KeyChord::new(0xA4, 0)).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(150));

    RUNNING.store(false, Ordering::Relaxed);
    std::thread::sleep(std::time::Duration::from_millis(100));
    println!("=== LL 钩子视角（其他工具看到的事件）===");
    for event in EVENTS.lock().unwrap().iter() {
        println!("  {event}");
    }
    std::process::exit(0);
}
