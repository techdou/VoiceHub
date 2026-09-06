//! 探针：注入鼠标侧键 XBUTTON1（SayIt 装有 WH_MOUSE_LL，日志称支持侧键触发）。

use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT,
};

fn mouse_input(xbutton: u32, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: xbutton,
                dwFlags: if up { MOUSEEVENTF_XUP } else { MOUSEEVENTF_XDOWN },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn main() {
    let xbutton1 = 0x0001;
    unsafe {
        let down = [mouse_input(xbutton1, false)];
        SendInput(&down, std::mem::size_of::<INPUT>() as i32);
        std::thread::sleep(std::time::Duration::from_millis(400));
        let up = [mouse_input(xbutton1, true)];
        SendInput(&up, std::mem::size_of::<INPUT>() as i32);
    }
    println!("XBUTTON1 tap done");
}
