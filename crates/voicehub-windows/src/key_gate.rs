//! F5 吞键闸（WH_KEYBOARD_LL）。
//!
//! 遥控器语音键在 HID 键盘层是 F5。原始 F5 若穿透到前台应用会触发
//! 刷新等行为，必须吞掉；但真实键盘的 F5 不能误伤。
//! 策略（与参考实现一致的防粘键设计）：
//! - DOWN 沿：会话激活、武装窗口内（GATT 控制通知后 ~250ms）或常驻武装
//!   （遥控器 BLE 已连接且总开关开启）任一成立才吞；非 F5 一律透传；
//!   注入事件（LLKHF_INJECTED）一律透传。
//! - 常驻武装是时序兜底的最终层：LL 钩子拿不到按键来源设备，GATT 通知
//!   缺失/迟到时首个 F5 仍会泄漏（真机实测结论，2026-09-12 前的日志可证）；
//!   遥控器在线期间直接吞掉全部 F5，代价是真键盘 F5 暂时失效（Ctrl+R 不受影响）。
//! - UP 沿：只按配对裁决——本次按住的所有 DOWN 全被吞才吞 UP，
//!   任何 DOWN 泄漏则 UP 必放行（宁送孤立 UP，不留 OS 粘键）。

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::time::Duration;

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED,
    LLKHF_LOWER_IL_INJECTED, MSG, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL,
};

const VK_F5: u32 = 0x74;

/// 武装窗口：GATT 控制通知到达后的一小段时间（遥控器 HID F5 通常
/// 晚 60–90ms 到达；应用被后台节流时工作线程可能再拖 120ms）。
const ARM_GRACE_MS: i64 = 250;

static MASTER: AtomicBool = AtomicBool::new(false);
static SESSION_ACTIVE: AtomicBool = AtomicBool::new(false);
static ARMED_UNTIL_MS: AtomicI64 = AtomicI64::new(0);
static HOLD_PAIRING: AtomicU32 = AtomicU32::new(HOLD_NONE);
static HOOK: AtomicI64 = AtomicI64::new(0);
/// 常驻武装总开关（设置项 f5GateEnabled，默认开）。
static GATE_ENABLED: AtomicBool = AtomicBool::new(true);
/// 遥控器 BLE 连接状态（Ready 时常驻武装，断开即解除）。
static REMOTE_CONNECTED: AtomicBool = AtomicBool::new(false);

pub const HOLD_NONE: u32 = 0;
pub const HOLD_SWALLOWED_ALL: u32 = 1;
pub const HOLD_LEAKED: u32 = 2;

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 纯决策（单测覆盖）。`persistent` = 常驻武装生效中（开关开且遥控器在线）。
pub fn decide(
    vk_code: u32,
    is_key_up: bool,
    session: bool,
    armed_now: bool,
    hold_pairing: u32,
    persistent: bool,
) -> bool {
    if vk_code != VK_F5 {
        return false;
    }
    if is_key_up {
        return hold_pairing == HOLD_SWALLOWED_ALL;
    }
    session || armed_now || persistent
}

/// 纯状态转移：DOWN 裁决后更新配对。
pub fn track_down(hold_pairing: u32, down_swallowed: bool) -> u32 {
    if hold_pairing == HOLD_LEAKED {
        return HOLD_LEAKED;
    }
    if down_swallowed {
        HOLD_SWALLOWED_ALL
    } else {
        HOLD_LEAKED
    }
}

/// GATT 控制通知回调线程直接调用（不要排队到工作线程——节流会迟到）。
pub fn arm_grace() {
    ARMED_UNTIL_MS.store(now_ms() + ARM_GRACE_MS, Ordering::Relaxed);
}

pub fn set_session_active(active: bool) {
    SESSION_ACTIVE.store(active, Ordering::Relaxed);
    if !active {
        // 会话结束：清配对，允许武装窗口继续兜尾沿。
        HOLD_PAIRING.store(HOLD_NONE, Ordering::Relaxed);
    }
}

/// 常驻武装总开关（设置项同步；关闭时退回纯时序兜底，真键盘 F5 恢复）。
pub fn set_gate_enabled(enabled: bool) {
    GATE_ENABLED.store(enabled, Ordering::Relaxed);
}

/// 遥控器 BLE 连接状态（BLE 快照变化时同步；Ready = 常驻武装）。
pub fn set_remote_connected(connected: bool) {
    REMOTE_CONNECTED.store(connected, Ordering::Relaxed);
}

fn persistent_armed() -> bool {
    GATE_ENABLED.load(Ordering::Relaxed) && REMOTE_CONNECTED.load(Ordering::Relaxed)
}

fn armed() -> bool {
    let until = ARMED_UNTIL_MS.load(Ordering::Relaxed);
    until != 0 && now_ms() < until
}

/// 安装全局钩子（启动专用线程跑消息泵；LL 钩子要求有线程消息循环）。
/// 消息泵退出或线程 panic 时主动卸载钩子并清 HOOK 标志——宿主
/// （bridge 周期自检）据 is_installed()=false 重装，F5 保护不静默失效。
pub fn install() -> bool {
    if HOOK.load(Ordering::Relaxed) != 0 {
        return true;
    }
    MASTER.store(true, Ordering::Relaxed);
    std::thread::Builder::new()
        .name("vh-key-gate".into())
        .spawn(|| {
            let pumped = std::panic::catch_unwind(|| unsafe {
                match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0) {
                    Ok(hook) => {
                        HOOK.store(hook.0 as i64, Ordering::Relaxed);
                    }
                    Err(error) => {
                        log::error!("SetWindowsHookExW(WH_KEYBOARD_LL) failed: {error}");
                        return;
                    }
                }
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    DispatchMessageW(&msg);
                }
                // 消息泵停转（WM_QUIT / GetMessageW 出错）：卸载并清标志。
                let handle = HOOK.swap(0, Ordering::Relaxed);
                if handle != 0 {
                    let hook = HHOOK(handle as *mut core::ffi::c_void);
                    let _ = UnhookWindowsHookEx(hook);
                }
            });
            if pumped.is_err() {
                // panic 路径兜底清标志（swap 卸载可能没跑到），
                // 留着旧值会让 is_installed() 误报、自检永不重装。
                HOOK.store(0, Ordering::Relaxed);
                log::error!("[key-gate] hook thread panicked; flag cleared for self-heal");
            }
        })
        .is_ok()
}

/// 钩子是否已安装（诊断用）。
pub fn is_installed() -> bool {
    HOOK.load(Ordering::Relaxed) != 0
}

/// 卸载并放行（应用退出 / 遥控器断开时）。
pub fn shutdown() {
    MASTER.store(false, Ordering::Relaxed);
    let handle = HOOK.swap(0, Ordering::Relaxed);
    if handle != 0 {
        unsafe {
            let hook = HHOOK(handle as *mut core::ffi::c_void);
            let _ = UnhookWindowsHookEx(hook);
        }
    }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{CallNextHookEx, WM_KEYDOWN, WM_SYSKEYDOWN};
    if code >= 0 && MASTER.load(Ordering::Relaxed) {
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let is_injected =
            (kb.flags & LLKHF_INJECTED).0 != 0 || (kb.flags & LLKHF_LOWER_IL_INJECTED).0 != 0;
        if !is_injected {
            let is_up = !(wparam.0 as u32 == WM_KEYDOWN || wparam.0 as u32 == WM_SYSKEYDOWN);
            let pairing = HOLD_PAIRING.load(Ordering::Relaxed);
            let swallow = decide(
                kb.vkCode as u32,
                is_up,
                SESSION_ACTIVE.load(Ordering::Relaxed),
                armed(),
                pairing,
                persistent_armed(),
            );
            if swallow {
                if !is_up {
                    // 吞 DOWN 即顺延武装窗口：长按语音键时键盘自动重复的 F5
                    // 会逐个到达，一次性 250ms 窗口过期后就泄漏（免提模式压掉
                    // 蓝牙流、无会话兜底时尤甚）——每吞一个续一窗，直至 UP。
                    arm_grace();
                    HOLD_PAIRING.store(track_down(pairing, true), Ordering::Relaxed);
                } else {
                    HOLD_PAIRING.store(HOLD_NONE, Ordering::Relaxed);
                }
                return LRESULT(1);
            }
            if !is_up && kb.vkCode as u32 == VK_F5 {
                // 泄漏只在配对状态转换时记一次（长按的重复 F5 不刷屏）。
                // 竞态成因：F5 走 HID 通道，控制通知走 GATT 通道，F5 先到则
                // 武装窗口未开——此时前台（如浏览器）会收到真实 F5（刷新）。
                if pairing != HOLD_LEAKED {
                    log::warn!(
                        "[key-gate] remote F5 leaked (arrived before/without arming); \
                         foreground may receive a refresh"
                    );
                }
                HOLD_PAIRING.store(track_down(pairing, false), Ordering::Relaxed);
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

/// 防粘键兜底：长时间无活动后重置配对（钩子中途装上等场景）。
pub fn reset_pairing_after_idle(idle: Duration) {
    let _ = idle; // 当前实现由 set_session_active(false) 清配对；保留接口对齐宿主。
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_f5_down_swallowed_when_armed_or_session() {
        assert!(!decide(0x41, false, false, false, HOLD_NONE, false));
        assert!(!decide(0x74, false, false, false, HOLD_NONE, false));
        assert!(decide(0x74, false, true, false, HOLD_NONE, false));
        assert!(decide(0x74, false, false, true, HOLD_NONE, false));
    }

    #[test]
    fn persistent_arming_swallows_despite_lost_timing_signals() {
        // 常驻武装：GATT 通知缺失/迟到、无会话时，F5 DOWN 仍被吞——
        // 这是"首个 F5 泄漏刷新页面"竞态的根治层。
        assert!(decide(0x74, false, false, false, HOLD_NONE, true));
        // 配对语义保持：全吞的按住 → 吞 UP。
        assert!(decide(0x74, true, false, false, HOLD_SWALLOWED_ALL, true));
        // 非 F5 不受常驻武装影响。
        assert!(!decide(0x41, false, false, false, HOLD_NONE, true));
    }

    #[test]
    fn up_edge_follows_down_pairing() {
        // 全吞的按住 → 吞 UP。
        assert!(decide(0x74, true, false, false, HOLD_SWALLOWED_ALL, false));
        // 任一 DOWN 泄漏 / 配对未知 → 放行 UP，即使会话仍激活。
        assert!(!decide(0x74, true, true, true, HOLD_LEAKED, false));
        assert!(!decide(0x74, true, true, true, HOLD_NONE, false));
        // 非 F5 的 UP 永不吞。
        assert!(!decide(0x41, true, true, true, HOLD_SWALLOWED_ALL, false));
    }

    #[test]
    fn leaked_hold_stays_leaked() {
        assert_eq!(track_down(HOLD_SWALLOWED_ALL, true), HOLD_SWALLOWED_ALL);
        assert_eq!(track_down(HOLD_SWALLOWED_ALL, false), HOLD_LEAKED);
        assert_eq!(track_down(HOLD_LEAKED, true), HOLD_LEAKED);
        assert_eq!(track_down(HOLD_NONE, false), HOLD_LEAKED);
    }
}
