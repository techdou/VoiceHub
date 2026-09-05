//! 语音会话状态机：Idle → Streaming → Draining → Idle。
//!
//! 代际（generation）单调递增，防止迟到的 Drain 完成把新会话错误归零；
//! session_id 校验防止上一会话的 StreamStop 干扰当前流。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceSessionState {
    Idle,
    Streaming,
    Draining,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceSessionEvent {
    StreamStarted { session_id: u8 },
    AudioAccepted { sample_count: usize },
    StreamStopped { session_id: u8 },
    DrainCompleted { generation: u64 },
    Interrupted,
}

#[derive(Debug, Clone)]
pub struct VoiceSession {
    state: VoiceSessionState,
    generation: u64,
    session_id: Option<u8>,
    accepted_samples: u64,
    started_at_ms: Option<i64>,
}

impl Default for VoiceSession {
    fn default() -> Self {
        Self {
            state: VoiceSessionState::Idle,
            generation: 0,
            session_id: None,
            accepted_samples: 0,
            started_at_ms: None,
        }
    }
}

impl VoiceSession {
    pub fn state(&self) -> VoiceSessionState {
        self.state
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn session_id(&self) -> Option<u8> {
        self.session_id
    }

    pub fn accepted_samples(&self) -> u64 {
        self.accepted_samples
    }

    pub fn started_at_ms(&self) -> Option<i64> {
        self.started_at_ms
    }

    pub fn apply(&mut self, event: VoiceSessionEvent, now_ms: i64) -> Result<(), VoiceSessionError> {
        match event {
            VoiceSessionEvent::StreamStarted { session_id } => {
                if self.state != VoiceSessionState::Idle {
                    return Err(VoiceSessionError::AlreadyActive);
                }
                self.generation = self.generation.wrapping_add(1);
                self.session_id = Some(session_id);
                self.accepted_samples = 0;
                self.started_at_ms = Some(now_ms);
                self.state = VoiceSessionState::Streaming;
            }
            VoiceSessionEvent::AudioAccepted { sample_count } => {
                if self.state != VoiceSessionState::Streaming {
                    return Err(VoiceSessionError::AudioOutsideStream);
                }
                self.accepted_samples = self.accepted_samples.saturating_add(sample_count as u64);
            }
            VoiceSessionEvent::StreamStopped { session_id } => {
                if self.state != VoiceSessionState::Streaming {
                    return Err(VoiceSessionError::NotStreaming);
                }
                if self.session_id != Some(session_id) {
                    return Err(VoiceSessionError::StaleSession);
                }
                self.state = VoiceSessionState::Draining;
            }
            VoiceSessionEvent::DrainCompleted { generation } => {
                if self.state != VoiceSessionState::Draining {
                    return Err(VoiceSessionError::NotDraining);
                }
                if generation != self.generation {
                    return Err(VoiceSessionError::StaleGeneration);
                }
                self.finish();
            }
            VoiceSessionEvent::Interrupted => self.finish(),
        }
        Ok(())
    }

    fn finish(&mut self) {
        self.state = VoiceSessionState::Idle;
        self.session_id = None;
        self.accepted_samples = 0;
        self.started_at_ms = None;
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VoiceSessionError {
    #[error("a voice session is already active")]
    AlreadyActive,
    #[error("audio arrived outside an active stream")]
    AudioOutsideStream,
    #[error("voice session is not streaming")]
    NotStreaming,
    #[error("voice session is not draining")]
    NotDraining,
    #[error("stream stop belongs to another session")]
    StaleSession,
    #[error("drain completion belongs to an earlier generation")]
    StaleGeneration,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_drain_before_returning_to_idle() {
        let mut s = VoiceSession::default();
        s.apply(VoiceSessionEvent::StreamStarted { session_id: 7 }, 0).unwrap();
        s.apply(VoiceSessionEvent::AudioAccepted { sample_count: 160 }, 10).unwrap();
        s.apply(VoiceSessionEvent::StreamStopped { session_id: 7 }, 200).unwrap();
        assert_eq!(s.state(), VoiceSessionState::Draining);
        let g = s.generation();
        s.apply(VoiceSessionEvent::DrainCompleted { generation: g }, 380).unwrap();
        assert_eq!(s.state(), VoiceSessionState::Idle);
    }

    #[test]
    fn stale_stop_cannot_end_current_session() {
        let mut s = VoiceSession::default();
        s.apply(VoiceSessionEvent::StreamStarted { session_id: 2 }, 0).unwrap();
        assert_eq!(
            s.apply(VoiceSessionEvent::StreamStopped { session_id: 1 }, 5),
            Err(VoiceSessionError::StaleSession)
        );
        assert_eq!(s.state(), VoiceSessionState::Streaming);
    }

    #[test]
    fn stale_generation_drain_is_rejected() {
        let mut s = VoiceSession::default();
        s.apply(VoiceSessionEvent::StreamStarted { session_id: 1 }, 0).unwrap();
        s.apply(VoiceSessionEvent::StreamStopped { session_id: 1 }, 9).unwrap();
        assert_eq!(
            s.apply(VoiceSessionEvent::DrainCompleted { generation: 0 }, 20),
            Err(VoiceSessionError::StaleGeneration)
        );
    }

    #[test]
    fn interruption_releases_every_state() {
        let mut s = VoiceSession::default();
        s.apply(VoiceSessionEvent::StreamStarted { session_id: 9 }, 0).unwrap();
        s.apply(VoiceSessionEvent::Interrupted, 1).unwrap();
        assert_eq!(s.state(), VoiceSessionState::Idle);
    }

    #[test]
    fn second_start_while_active_is_rejected() {
        let mut s = VoiceSession::default();
        s.apply(VoiceSessionEvent::StreamStarted { session_id: 3 }, 0).unwrap();
        assert_eq!(
            s.apply(VoiceSessionEvent::StreamStarted { session_id: 3 }, 1),
            Err(VoiceSessionError::AlreadyActive)
        );
    }

    #[test]
    fn tracks_start_time_and_samples() {
        let mut s = VoiceSession::default();
        s.apply(VoiceSessionEvent::StreamStarted { session_id: 5 }, 1234).unwrap();
        s.apply(VoiceSessionEvent::AudioAccepted { sample_count: 16_000 }, 1300).unwrap();
        assert_eq!(s.started_at_ms(), Some(1234));
        assert_eq!(s.accepted_samples(), 16_000);
    }
}
