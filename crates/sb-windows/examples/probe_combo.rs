//! 探针：注入组合键 Ctrl+Alt+H（SayIt 免提热键，走 RegisterHotKey 通道，
//! 该通道不区分注入与物理按键）。

use sb_windows::send_input::{tap, KeyChord};

fn main() {
    let chord = KeyChord::new(0x48, sb_core::actions::MOD_CONTROL | sb_core::actions::MOD_ALT);
    println!("注入 Ctrl+Alt+H（第一次：开始录音）...");
    println!("tap = {:?}", tap(chord));
    std::thread::sleep(std::time::Duration::from_millis(1200));
    println!("注入 Ctrl+Alt+H（第二次：停止录音）...");
    println!("tap = {:?}", tap(chord));
    std::thread::sleep(std::time::Duration::from_millis(800));
    println!("done");
}
