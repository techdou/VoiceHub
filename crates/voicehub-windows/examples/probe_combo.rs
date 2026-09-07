//! 探针：注入组合键（参数默认 Alt+V），观察谁响应。

use voicehub_windows::send_input::{tap, KeyChord};

fn main() {
    let chord = KeyChord::new(0x56, voicehub_core::actions::MOD_ALT);
    println!("注入 Alt+V ...");
    println!("tap = {:?}", tap(chord));
    std::thread::sleep(std::time::Duration::from_millis(800));
    println!("done");
}
