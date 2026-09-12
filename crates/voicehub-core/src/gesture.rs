//! 按键手势识别：单击 / 双击 / 长按。
//!
//! 计时与参考实现一致：双击窗口 300 ms、长按阈值 550 ms。
//! 纯状态机，由调用方喂入按下/释放沿与单调时钟。

use std::collections::HashMap;
use std::time::Duration;

use crate::buttons::RemoteButton;

pub const DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(300);
pub const LONG_PRESS_THRESHOLD: Duration = Duration::from_millis(550);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gesture {
    /// 短按释放（且无双击配对）。
    SingleClick,
    DoubleClick,
    LongPress,
    /// 长按期间的自动重复（按住方向键连发）。
    Repeat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GestureEvent {
    pub button: RemoteButton,
    pub gesture: Gesture,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ButtonTracking {
    /// 等待释放：释放时若未超长按阈值 → 进入双击窗口。
    Held { pressed_at_ms: u64 },
    /// 双击配对的第二次按下（DoubleClick 已发）：释放直接清状态。
    HeldAfterDouble { pressed_at_ms: u64 },
    /// 已释放一次，等待第二次按下或窗口超时（发单击）。
    WaitingSecond { released_at_ms: u64 },
}

pub struct GestureRecognizer {
    tracking: HashMap<RemoteButton, ButtonTracking>,
    double_click_window: Duration,
    long_press_threshold: Duration,
    repeat_interval: Duration,
    /// 上次长按/连发触发时刻（按键粒度）。
    last_fire: HashMap<RemoteButton, u64>,
}

impl Default for GestureRecognizer {
    fn default() -> Self {
        Self::new()
    }
}

impl GestureRecognizer {
    pub fn new() -> Self {
        Self {
            tracking: HashMap::new(),
            double_click_window: DOUBLE_CLICK_WINDOW,
            long_press_threshold: LONG_PRESS_THRESHOLD,
            repeat_interval: Duration::from_millis(200),
            last_fire: HashMap::new(),
        }
    }

    /// 按下沿。
    pub fn press(&mut self, button: RemoteButton, now_ms: u64) -> Vec<GestureEvent> {
        let mut events = Vec::new();
        let mut from_double = false;
        if let Some(ButtonTracking::WaitingSecond { released_at_ms }) = self.tracking.remove(&button) {
            if now_ms.saturating_sub(released_at_ms) <= self.double_click_window.as_millis() as u64
            {
                events.push(GestureEvent { button, gesture: Gesture::DoubleClick });
                from_double = true;
            } else {
                // 窗口已超时：先补发单击，再按新按下处理。
                events.push(GestureEvent { button, gesture: Gesture::SingleClick });
            }
        }
        self.tracking.insert(
            button,
            if from_double {
                ButtonTracking::HeldAfterDouble { pressed_at_ms: now_ms }
            } else {
                ButtonTracking::Held { pressed_at_ms: now_ms }
            },
        );
        self.last_fire.remove(&button);
        events
    }

    /// 释放沿。
    pub fn release(&mut self, button: RemoteButton, now_ms: u64) -> Vec<GestureEvent> {
        let events = Vec::new();
        match self.tracking.remove(&button) {
            Some(ButtonTracking::Held { pressed_at_ms }) => {
                let held = now_ms.saturating_sub(pressed_at_ms);
                if held < self.long_press_threshold.as_millis() as u64 {
                    self.tracking
                        .insert(button, ButtonTracking::WaitingSecond { released_at_ms: now_ms });
                }
            }
            // 双击第二次释放：不进入单击等待窗。
            Some(ButtonTracking::HeldAfterDouble { .. }) | Some(ButtonTracking::WaitingSecond { .. }) | None => {}
        }
        self.last_fire.remove(&button);
        events
    }

    /// 时钟推进：触发长按、按住连发、双击窗口超时（补发单击）。
    pub fn tick(&mut self, now_ms: u64) -> Vec<GestureEvent> {
        let mut events = Vec::new();
        let buttons: Vec<RemoteButton> = self.tracking.keys().copied().collect();
        for button in buttons {
            let (pressed_at_ms, from_double) = match self.tracking.get(&button).copied() {
                Some(ButtonTracking::Held { pressed_at_ms }) => (pressed_at_ms, false),
                Some(ButtonTracking::HeldAfterDouble { pressed_at_ms }) => (pressed_at_ms, true),
                Some(ButtonTracking::WaitingSecond { released_at_ms }) => {
                    let since = now_ms.saturating_sub(released_at_ms);
                    if since > self.double_click_window.as_millis() as u64 {
                        self.tracking.remove(&button);
                        events.push(GestureEvent { button, gesture: Gesture::SingleClick });
                    }
                    continue;
                }
                None => continue,
            };
            let held = now_ms.saturating_sub(pressed_at_ms);
            let threshold = self.long_press_threshold.as_millis() as u64;
            // 双击配对的第二次按住也允许长按二级动作。
            if held >= threshold {
                let last = self.last_fire.get(&button).copied();
                match last {
                    None => {
                        self.last_fire.insert(button, now_ms);
                        events.push(GestureEvent { button, gesture: Gesture::LongPress });
                    }
                    Some(last_ms) => {
                        let repeat_every = self.repeat_interval.as_millis() as u64;
                        if now_ms.saturating_sub(last_ms) >= repeat_every {
                            self.last_fire.insert(button, now_ms);
                            events.push(GestureEvent { button, gesture: Gesture::Repeat });
                        }
                    }
                }
            }
            let _ = from_double;
        }
        events
    }

    /// 连接断开等场景：清空全部挂起状态（不发事件）。
    pub fn reset(&mut self) {
        self.tracking.clear();
        self.last_fire.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buttons::RemoteButton::*;

    /// 按时间轴逐拍（10ms）推进，在精确时刻插入按下/释放沿。
    fn fire(recognizer: &mut GestureRecognizer, sequence: &[(u64, RemoteButton, bool)]) -> Vec<GestureEvent> {
        let mut all = Vec::new();
        let mut events_by_time: std::collections::BTreeMap<u64, Vec<(RemoteButton, bool)>> =
            Default::default();
        let mut last = 0u64;
        for &(t, button, pressed) in sequence {
            events_by_time.entry(t).or_default().push((button, pressed));
            last = last.max(t);
        }
        for t in (0..=last + 2000).step_by(10) {
            if let Some(ops) = events_by_time.remove(&t) {
                for (button, pressed) in ops {
                    let e = if pressed {
                        recognizer.press(button, t)
                    } else {
                        recognizer.release(button, t)
                    };
                    all.extend(e);
                }
            }
            all.extend(recognizer.tick(t));
        }
        all
    }

    #[test]
    fn single_click_resolved_after_window() {
        let mut r = GestureRecognizer::new();
        let events = fire(&mut r, &[(100, Ok, true), (180, Ok, false)]);
        assert_eq!(events, vec![GestureEvent { button: Ok, gesture: Gesture::SingleClick }]);
    }

    #[test]
    fn fast_second_press_is_double_click() {
        let mut r = GestureRecognizer::new();
        let events = fire(
            &mut r,
            &[(100, Home, true), (160, Home, false), (350, Home, true), (420, Home, false)],
        );
        assert_eq!(events, vec![GestureEvent { button: Home, gesture: Gesture::DoubleClick }]);
    }

    #[test]
    fn slow_second_press_is_two_single_clicks() {
        let mut r = GestureRecognizer::new();
        let events = fire(
            &mut r,
            &[(100, Ok, true), (160, Ok, false), (900, Ok, true), (960, Ok, false)],
        );
        assert_eq!(
            events,
            vec![
                GestureEvent { button: Ok, gesture: Gesture::SingleClick },
                GestureEvent { button: Ok, gesture: Gesture::SingleClick },
            ]
        );
    }

    #[test]
    fn long_hold_fires_long_press_not_single() {
        let mut r = GestureRecognizer::new();
        let events = fire(&mut r, &[(100, Menu, true), (800, Menu, false)]);
        assert_eq!(events, vec![GestureEvent { button: Menu, gesture: Gesture::LongPress }]);
    }

    #[test]
    fn hold_below_threshold_is_single_click() {
        let mut r = GestureRecognizer::new();
        let events = fire(&mut r, &[(100, Menu, true), (600, Menu, false)]);
        // 500ms < 550ms 阈值 → 单击。
        assert_eq!(events, vec![GestureEvent { button: Menu, gesture: Gesture::SingleClick }]);
    }

    #[test]
    fn independent_buttons_do_not_interfere() {
        let mut r = GestureRecognizer::new();
        let events = fire(
            &mut r,
            &[(100, Ok, true), (120, Menu, true), (160, Menu, false), (170, Ok, false)],
        );
        assert!(events.contains(&GestureEvent { button: Ok, gesture: Gesture::SingleClick }));
        assert!(events.contains(&GestureEvent { button: Menu, gesture: Gesture::SingleClick }));
    }

    #[test]
    fn repeat_emits_while_held() {
        let mut r = GestureRecognizer::new();
        r.press(Menu, 0);
        assert!(r.tick(600).contains(&GestureEvent { button: Menu, gesture: Gesture::LongPress }));
        let repeats: Vec<_> = ((600..1200usize).step_by(8))
            .flat_map(|t| r.tick(t as u64))
            .filter(|e| e.gesture == Gesture::Repeat)
            .collect();
        assert!(!repeats.is_empty(), "expected repeat events during hold");
    }
}
