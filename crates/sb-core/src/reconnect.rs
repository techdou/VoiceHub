//! 指数退避重连策略（2–30 秒 + 抖动；缓存目标直连失败后绕过缓存改走扫描）。

use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ReconnectPolicy {
    consecutive_failures: u32,
    min: Duration,
    max: Duration,
    /// 连续直连缓存目标失败次数达到阈值后，本轮绕过缓存直接扫描。
    cached_bypass_after: u32,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            consecutive_failures: 0,
            min: Duration::from_secs(2),
            max: Duration::from_secs(30),
            cached_bypass_after: 1,
        }
    }
}

impl ReconnectPolicy {
    pub fn new(min: Duration, max: Duration) -> Self {
        Self { min, max, ..Default::default() }
    }

    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    pub fn reset(&mut self) {
        self.consecutive_failures = 0;
    }

    /// 记录一次失败并取下一次延迟。`jitter_unit` ∈ [0,1)。
    pub fn next_delay(&mut self, jitter_unit: f64) -> Duration {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        let exp = (self.consecutive_failures - 1).min(16) as f64;
        let factor = 2f64.powf(exp);
        let base = (self.min.as_millis() as f64 * factor).min(self.max.as_millis() as f64);
        let jitter = 1.0 + jitter_unit.clamp(0.0, 0.999) * 0.2;
        Duration::from_millis((base * jitter) as u64)
    }

    /// 是否允许本轮先用缓存标识直连。首次失败后即绕过，
    /// 防止失效的缓存地址反复占用连接窗口。
    pub fn allows_cached_target(&self) -> bool {
        if self.consecutive_failures == 0 {
            return true;
        }
        // 成功过一次能力协商后 consecutive_failures 归零；
        // 之后偶发断连允许再试一次缓存直连。
        self.consecutive_failures < self.cached_bypass_after
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_grow_exponentially_then_cap() {
        let mut policy = ReconnectPolicy::new(Duration::from_secs(2), Duration::from_secs(30));
        assert_eq!(policy.next_delay(0.0), Duration::from_secs(2));
        let second = policy.next_delay(0.0);
        assert_eq!(second, Duration::from_secs(4));
        for _ in 0..10 {
            let d = policy.next_delay(0.0);
            assert!(d <= Duration::from_millis(30_000 * 12 / 10));
        }
        let capped = policy.next_delay(0.99);
        assert!(capped <= Duration::from_millis(36_000), "capped+20% jitter, got {capped:?}");
    }

    #[test]
    fn reset_restores_cached_target() {
        let mut policy = ReconnectPolicy::default();
        policy.next_delay(0.0);
        assert!(!policy.allows_cached_target());
        policy.reset();
        assert!(policy.allows_cached_target());
    }

    #[test]
    fn jitter_keeps_delay_in_band() {
        let mut policy = ReconnectPolicy::default();
        let d = policy.next_delay(0.5);
        assert!(d >= Duration::from_secs(2) && d < Duration::from_millis(2_400));
    }
}
