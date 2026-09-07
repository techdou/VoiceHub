//! ATVV 音频字节流 → 定长帧累积器。
//!
//! BLE 通知不保证帧边界；按能力协商的 frame_size 切片，
//! 流开始 / 解码器同步时丢弃残帧。

#[derive(Debug, Clone, Default)]
pub struct FrameAccumulator {
    pending: Vec<u8>,
}

impl FrameAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// 追加数据，返回完整切出的帧（按 `frame_size` 对齐）。
    pub fn append(&mut self, data: &[u8], frame_size: usize) -> Vec<Vec<u8>> {
        if frame_size == 0 {
            return Vec::new();
        }
        self.pending.extend_from_slice(data);
        let mut frames = Vec::new();
        while self.pending.len() >= frame_size {
            let frame: Vec<u8> = self.pending.drain(..frame_size).collect();
            frames.push(frame);
        }
        frames
    }

    /// 丢弃残帧（流开始 / 同步时调用），返回丢弃的字节数。
    pub fn reset(&mut self) -> usize {
        let dropped = self.pending.len();
        self.pending.clear();
        dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulates_and_splits_frames() {
        let mut acc = FrameAccumulator::new();
        let first = acc.append(&[0u8; 100], 120);
        assert!(first.is_empty());
        assert_eq!(acc.pending_len(), 100);
        let second = acc.append(&[0u8; 150], 120);
        assert_eq!(second.len(), 2);
        assert_eq!(acc.pending_len(), 10);
    }

    #[test]
    fn reset_reports_dropped_partial() {
        let mut acc = FrameAccumulator::new();
        acc.append(&[1, 2, 3, 4, 5], 120);
        assert_eq!(acc.reset(), 5);
        assert_eq!(acc.reset(), 0);
    }

    #[test]
    fn zero_frame_size_ignores_data() {
        let mut acc = FrameAccumulator::new();
        assert!(acc.append(&[1, 2, 3], 0).is_empty());
        // 帧长 0 属于无效协商：丢弃数据，避免 pending 无界增长。
        assert_eq!(acc.pending_len(), 0);
    }
}
