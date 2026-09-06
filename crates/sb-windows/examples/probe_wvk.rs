//! 探针：wVk 方式注入 VK_RMENU（对照声桥的 KEYEVENTF_SCANCODE 方式）。
//! 变体 A1 = wVk + EXTENDEDKEY；A2 = wVk 无扩展位。间隔 1.5s 便于日志区分。

use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VIRTUAL_KEY,
};

fn wvk_input(vk: u16, extended: bool, up: bool) -> INPUT {
    let mut flags = KEYBD_EVENT_FLAGS(0);
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
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn main() {
    println!("A1: wVk=0xA5 + EXTENDED ...");
    unsafe {
        let down = [wvk_input(0xA5, true, false)];
        SendInput(&down, std::mem::size_of::<INPUT>() as i32);
        std::thread::sleep(std::time::Duration::from_millis(60));
        let up = [wvk_input(0xA5, true, true)];
        SendInput(&up, std::mem::size_of::<INPUT>() as i32);
    }
    std::thread::sleep(std::time::Duration::from_millis(1500));
    println!("A2: wVk=0xA5 无扩展位 ...");
    unsafe {
        let down = [wvk_input(0xA5, false, false)];
        SendInput(&down, std::mem::size_of::<INPUT>() as i32);
        std::thread::sleep(std::time::Duration::from_millis(60));
        let up = [wvk_input(0xA5, false, true)];
        SendInput(&up, std::mem::size_of::<INPUT>() as i32);
    }
    std::thread::sleep(std::time::Duration::from_millis(800));
    println!("done");
}
