//! 探针：注入 Win+H（Windows 系统听写）。验证系统级组件是否接受注入
//! （与 SayIt 的注入过滤对照）。

use sb_windows::send_input::{tap, KeyChord};

fn main() {
    println!("注入 Win+H ...");
    let result = tap(KeyChord::new(0x48, sb_core::actions::MOD_WIN));
    println!("tap(Win+H) = {result:?}");
    std::thread::sleep(std::time::Duration::from_millis(2500));
    println!("done（观察屏幕是否出现听写浮条）");
}
