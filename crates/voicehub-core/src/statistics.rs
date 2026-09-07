//! 使用统计：按日桶聚合按键次数、语音会话数与时长。
//!
//! 只存本机（app_data_dir/statistics.json），无任何上报。

use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, Duration as ChronoDuration, Local};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DailyUsage {
    /// 按键总次数（沿触发计数，含长按/双击的触发沿）。
    pub button_press_count: u64,
    /// 按键粒度计数（按键 ID → 次数）。
    pub button_counts: BTreeMap<String, u64>,
    pub voice_session_count: u64,
    /// 语音累计毫秒。
    pub voice_duration_ms: u64,
    /// 最长单次语音（毫秒）。
    pub longest_voice_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UsageStatistics {
    /// 本地日期字符串 YYYY-MM-DD → 桶。
    pub days: BTreeMap<String, DailyUsage>,
}

/// 统计事件（宿主喂入）。
#[derive(Debug, Clone, PartialEq)]
pub enum UsageEvent {
    ButtonPress { button_id: String },
    VoiceSession { duration_ms: u64 },
}

impl UsageStatistics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, event: UsageEvent, at: DateTime<Local>) {
        let key = at.format("%Y-%m-%d").to_string();
        let day = self.days.entry(key).or_default();
        match event {
            UsageEvent::ButtonPress { button_id } => {
                day.button_press_count += 1;
                *day.button_counts.entry(button_id).or_insert(0) += 1;
            }
            UsageEvent::VoiceSession { duration_ms } => {
                day.voice_session_count += 1;
                day.voice_duration_ms += duration_ms;
                day.longest_voice_ms = day.longest_voice_ms.max(duration_ms);
            }
        }
    }

    pub fn total_button_presses(&self) -> u64 {
        self.days.values().map(|d| d.button_press_count).sum()
    }

    pub fn total_voice_sessions(&self) -> u64 {
        self.days.values().map(|d| d.voice_session_count).sum()
    }

    pub fn total_voice_duration_ms(&self) -> u64 {
        self.days.values().map(|d| d.voice_duration_ms).sum()
    }

    pub fn longest_voice_ms(&self) -> u64 {
        self.days.values().map(|d| d.longest_voice_ms).max().unwrap_or(0)
    }

    /// 今日桶。
    pub fn today(&self, now: DateTime<Local>) -> DailyUsage {
        let key = now.format("%Y-%m-%d").to_string();
        self.days.get(&key).cloned().unwrap_or_default()
    }

    /// 本周（周一起）聚合。
    pub fn week(&self, now: DateTime<Local>) -> DailyUsage {
        let days_from_monday = (now.weekday().num_days_from_monday()) as i64;
        self.aggregate_range(now - ChronoDuration::days(days_from_monday), now)
    }

    /// 近 7 天（含今日）聚合。
    pub fn recent_7_days(&self, now: DateTime<Local>) -> DailyUsage {
        self.aggregate_range(now - ChronoDuration::days(6), now)
    }

    pub fn all(&self) -> DailyUsage {
        let mut total = DailyUsage::default();
        for day in self.days.values() {
            total.button_press_count += day.button_press_count;
            for (k, v) in &day.button_counts {
                *total.button_counts.entry(k.clone()).or_insert(0) += v;
            }
            total.voice_session_count += day.voice_session_count;
            total.voice_duration_ms += day.voice_duration_ms;
            total.longest_voice_ms = total.longest_voice_ms.max(day.longest_voice_ms);
        }
        total
    }

    fn aggregate_range(&self, from: DateTime<Local>, to: DateTime<Local>) -> DailyUsage {
        let mut total = DailyUsage::default();
        let mut cursor = from.date_naive();
        let end = to.date_naive();
        while cursor <= end {
            let key = cursor.format("%Y-%m-%d").to_string();
            if let Some(day) = self.days.get(&key) {
                total.button_press_count += day.button_press_count;
                for (k, v) in &day.button_counts {
                    *total.button_counts.entry(k.clone()).or_insert(0) += v;
                }
                total.voice_session_count += day.voice_session_count;
                total.voice_duration_ms += day.voice_duration_ms;
                total.longest_voice_ms = total.longest_voice_ms.max(day.longest_voice_ms);
            }
            cursor += ChronoDuration::days(1);
        }
        total
    }

    /// 近 N 天的每日语音时长（毫秒），按日期升序，供 UI 画柱状图。
    pub fn daily_voice_series(&self, now: DateTime<Local>, days: usize) -> Vec<(String, u64)> {
        let mut series = Vec::with_capacity(days);
        for offset in (0..days as i64).rev() {
            let day = now - ChronoDuration::days(offset);
            let key = day.format("%Y-%m-%d").to_string();
            let label = day.format("%m-%d").to_string();
            let ms = self.days.get(&key).map(|d| d.voice_duration_ms).unwrap_or(0);
            series.push((label, ms));
        }
        series
    }

    /// 清理 N 天前的旧桶（隐私保留窗口）。
    pub fn prune_before(&mut self, now: DateTime<Local>, keep_days: i64) -> usize {
        let cutoff = (now - ChronoDuration::days(keep_days)).format("%Y-%m-%d").to_string();
        let before = self.days.len();
        self.days.retain(|k, _| k.as_str() >= cutoff.as_str());
        before - self.days.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn local(y: i32, m: u32, d: u32, h: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    #[test]
    fn accumulates_events_per_day() {
        let mut stats = UsageStatistics::new();
        let day = local(2026, 9, 1, 10);
        stats.apply(UsageEvent::ButtonPress { button_id: "up".into() }, day);
        stats.apply(UsageEvent::ButtonPress { button_id: "up".into() }, day);
        stats.apply(UsageEvent::VoiceSession { duration_ms: 2_000 }, day);
        stats.apply(UsageEvent::VoiceSession { duration_ms: 5_000 }, day);

        let today = stats.today(day);
        assert_eq!(today.button_press_count, 2);
        assert_eq!(today.button_counts.get("up"), Some(&2));
        assert_eq!(today.voice_session_count, 2);
        assert_eq!(today.voice_duration_ms, 7_000);
        assert_eq!(today.longest_voice_ms, 5_000);
    }

    #[test]
    fn week_aggregation_from_monday() {
        let mut stats = UsageStatistics::new();
        // 2026-09-03 是周四；周一为 08-31。
        stats.apply(UsageEvent::ButtonPress { button_id: "ok".into() }, local(2026, 8, 31, 9));
        stats.apply(UsageEvent::ButtonPress { button_id: "ok".into() }, local(2026, 9, 3, 9));
        // 上周日（08-30）不计入本周。
        stats.apply(UsageEvent::ButtonPress { button_id: "ok".into() }, local(2026, 8, 30, 9));

        let week = stats.week(local(2026, 9, 3, 12));
        assert_eq!(week.button_press_count, 2);
        assert_eq!(stats.total_button_presses(), 3);
    }

    #[test]
    fn recent_seven_days_includes_today() {
        let mut stats = UsageStatistics::new();
        stats.apply(UsageEvent::VoiceSession { duration_ms: 1_000 }, local(2026, 9, 6, 8));
        stats.apply(UsageEvent::VoiceSession { duration_ms: 1_000 }, local(2026, 8, 31, 8)); // 恰好第 7 天
        stats.apply(UsageEvent::VoiceSession { duration_ms: 1_000 }, local(2026, 8, 30, 8)); // 第 8 天，排除

        let recent = stats.recent_7_days(local(2026, 9, 6, 20));
        assert_eq!(recent.voice_session_count, 2);
        assert_eq!(recent.voice_duration_ms, 2_000);
    }

    #[test]
    fn daily_series_ordered_ascending() {
        let mut stats = UsageStatistics::new();
        stats.apply(UsageEvent::VoiceSession { duration_ms: 3_000 }, local(2026, 9, 6, 8));
        let series = stats.daily_voice_series(local(2026, 9, 6, 12), 3);
        assert_eq!(series.len(), 3);
        assert_eq!(series.first().unwrap().0, "09-04");
        assert_eq!(series.last().unwrap().1, 3_000);
    }

    #[test]
    fn prune_drops_old_days() {
        let mut stats = UsageStatistics::new();
        stats.apply(UsageEvent::ButtonPress { button_id: "ok".into() }, local(2026, 8, 1, 8));
        stats.apply(UsageEvent::ButtonPress { button_id: "ok".into() }, local(2026, 9, 6, 8));
        let dropped = stats.prune_before(local(2026, 9, 6, 12), 30);
        assert_eq!(dropped, 1);
        assert_eq!(stats.total_button_presses(), 1);
    }

    #[test]
    fn serialization_roundtrip() {
        let mut stats = UsageStatistics::new();
        stats.apply(UsageEvent::ButtonPress { button_id: "home".into() }, local(2026, 9, 5, 9));
        let json = serde_json::to_string(&stats).unwrap();
        let back: UsageStatistics = serde_json::from_str(&json).unwrap();
        assert_eq!(stats, back);
    }
}
