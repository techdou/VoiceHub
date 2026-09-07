//! 持久化：settings.json / statistics.json / history.jsonl（原子写 + .bak）。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use voicehub_core::settings::{AppSettings, VoiceSessionRecord};
use voicehub_core::statistics::UsageStatistics;

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn settings_path(&self) -> PathBuf {
        self.dir.join("settings.json")
    }

    fn statistics_path(&self) -> PathBuf {
        self.dir.join("statistics.json")
    }

    fn history_path(&self) -> PathBuf {
        self.dir.join("history.jsonl")
    }

    fn logs_dir(&self) -> PathBuf {
        self.dir.join("logs")
    }

    pub fn load_settings(&self) -> AppSettings {
        // 主文件损坏（半截写入/磁盘问题）时回退 .bak——atomic_write_json
        // 每次保存前都会备份，不回读等于白备份（全部键位配置无声丢失）。
        let path = self.settings_path();
        if let Ok(raw) = fs::read_to_string(&path) {
            match AppSettings::load(&raw) {
                Ok(settings) => return settings,
                Err(_) => {
                    if let Ok(bak) = fs::read_to_string(path.with_extension("bak")) {
                        if let Ok(settings) = AppSettings::load(&bak) {
                            log::warn!("settings.json 解析失败，已从 .bak 备份恢复");
                            return settings;
                        }
                    }
                    log::warn!("settings.json 解析失败且无可用 .bak，重置为默认设置");
                }
            }
        }
        AppSettings::default()
    }

    pub fn save_settings(&self, settings: &AppSettings) -> std::io::Result<()> {
        atomic_write_json(&self.settings_path(), &serde_json::to_string_pretty(settings)?)
    }

    pub fn load_statistics(&self) -> UsageStatistics {
        let path = self.statistics_path();
        if let Ok(raw) = fs::read_to_string(&path) {
            if let Ok(stats) = serde_json::from_str(&raw) {
                return stats;
            }
            if let Ok(bak) = fs::read_to_string(path.with_extension("bak")) {
                if let Ok(stats) = serde_json::from_str(&bak) {
                    log::warn!("statistics.json 解析失败，已从 .bak 备份恢复");
                    return stats;
                }
            }
        }
        UsageStatistics::default()
    }

    pub fn save_statistics(&self, stats: &UsageStatistics) -> std::io::Result<()> {
        atomic_write_json(&self.statistics_path(), &serde_json::to_string_pretty(stats)?)
    }

    /// 追加一条语音会话记录（jsonl，追加不重写）。
    pub fn append_history(&self, record: &VoiceSessionRecord) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let line = serde_json::to_string(record)? + "\n";
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.history_path())?;
        file.write_all(line.as_bytes())
    }

    /// 最近 N 条（新的在前）。
    pub fn load_history(&self, limit: usize) -> Vec<VoiceSessionRecord> {
        let Ok(raw) = fs::read_to_string(self.history_path()) else {
            return Vec::new();
        };
        let mut records: Vec<VoiceSessionRecord> = raw
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        records.reverse();
        records.truncate(limit);
        records
    }

    /// 清空历史。
    pub fn clear_history(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        fs::write(self.history_path(), "")
    }

    pub fn log_file(&self) -> PathBuf {
        let dir = self.logs_dir();
        let _ = fs::create_dir_all(&dir);
        dir.join(format!("voicehub-{}.log", chrono::Local::now().format("%Y%m%d")))
    }
}

/// 同目录原子替换：写 .tmp → 备份 .bak → rename。
fn atomic_write_json(path: &Path, content: &str) -> std::io::Result<()> {
    let parent = path.parent().expect("settings path has parent");
    fs::create_dir_all(parent)?;
    let tmp = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
    }
    if path.exists() {
        let bak = path.with_extension("bak");
        let _ = fs::copy(path, &bak);
    }
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_roundtrip_through_disk() {
        let dir = std::env::temp_dir().join(format!("vh-store-test-{}", std::process::id()));
        let store = Store::new(&dir);
        let mut settings = AppSettings::default();
        settings.gain_db = 3.5;
        store.save_settings(&settings).unwrap();
        let loaded = store.load_settings();
        assert_eq!(loaded.gain_db, 3.5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_appends_and_reads_newest_first() {
        let dir = std::env::temp_dir().join(format!("vh-history-test-{}", std::process::id()));
        let store = Store::new(&dir);
        for i in 0..3 {
            store
                .append_history(&VoiceSessionRecord {
                    started_at_ms: i,
                    duration_ms: 100,
                    sample_count: 1600,
                    foreground_process: Some("a".into()),
                    profile_name: "p".into(),
                })
                .unwrap();
        }
        let history = store.load_history(2);
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].started_at_ms, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
