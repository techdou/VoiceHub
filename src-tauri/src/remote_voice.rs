//! Bounded direct PCM handoff. The renderer acknowledges setup before draining audio.
use std::collections::VecDeque;

const MAX_SAMPLES: usize = 16_000 * 300;
const BATCH_SAMPLES: usize = 8_000;

#[derive(Default)]
pub struct RemoteVoice {
    session: Option<Session>,
    client: Option<String>,
}
struct Session {
    id: u64,
    created: u64,
    offered: bool,
    ready: bool,
    ended: bool,
    error: Option<String>,
    received: usize,
    samples: VecDeque<i16>,
}

#[derive(serde::Serialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Packet {
    Start { id: u64 },
    Audio { id: u64, samples: Vec<i16>, ended: bool, error: Option<String> },
}

impl RemoteVoice {
    pub fn attach(&mut self, client: &str) {
        if self.client.as_deref().is_some_and(|previous| previous != client) {
            self.session = None;
        }
        self.client = Some(client.to_owned());
    }
    pub fn begin(&mut self, id: u64, now: u64) -> bool {
        self.expire(now);
        if self.session.is_some() { return false; }
        self.session = Some(Session { id, created: now, offered: false, ready: false,
            ended: false, error: None, received: 0, samples: VecDeque::new() });
        true
    }
    pub fn push(&mut self, id: u64, samples: &[i16]) {
        let Some(s) = self.session.as_mut().filter(|s| s.id == id && !s.ended) else { return };
        if s.received + samples.len() > MAX_SAMPLES {
            s.error = Some("Remote recording exceeded five minutes".into());
            s.ended = true;
            return;
        }
        s.received += samples.len();
        s.samples.extend(samples);
    }
    pub fn end(&mut self, id: u64) {
        if let Some(s) = self.session.as_mut().filter(|s| s.id == id) { s.ended = true; }
    }
    pub fn cancel(&mut self, reason: &str) {
        if let Some(s) = self.session.as_mut() {
            s.ended = true;
            s.error = Some(reason.into());
            s.samples.clear();
        }
    }
    pub fn ack(&mut self, id: u64) -> bool {
        if let Some(s) = self.session.as_mut().filter(|s| s.id == id && s.offered) {
            s.ready = true;
            true
        } else { false }
    }
    pub fn release(&mut self, id: u64) {
        if self.session.as_ref().is_some_and(|s| s.id == id) { self.session = None; }
    }
    fn expire(&mut self, now: u64) {
        if self.session.as_ref().is_some_and(|s| now.saturating_sub(s.created) > 360_000 ||
            (!s.ready && now.saturating_sub(s.created) > 30_000)) { self.session = None; }
    }
    pub fn poll(&mut self, now: u64) -> Option<Packet> {
        if self.session.as_ref().is_some_and(|s| s.ready && now.saturating_sub(s.created) > 360_000) {
            self.cancel("Remote recording timed out");
            let s = self.session.as_ref()?;
            return Some(Packet::Audio { id: s.id, samples: vec![], ended: true, error: s.error.clone() });
        }
        self.expire(now);
        let s = self.session.as_mut()?;
        if !s.offered {
            s.offered = true;
            return Some(Packet::Start { id: s.id });
        }
        if !s.ready { return None; }
        let count = BATCH_SAMPLES.min(s.samples.len());
        if count == 0 && !s.ended { return None; }
        let samples = s.samples.drain(..count).collect();
        Some(Packet::Audio { id: s.id, samples, ended: s.ended && s.samples.is_empty(), error: s.error.clone() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn early_release_keeps_first_and_last_samples_until_ready() {
        let mut r = RemoteVoice::default();
        assert!(r.begin(1, 0));
        r.push(1, &[1, 2, 3]);
        r.end(1);
        assert!(matches!(r.poll(10), Some(Packet::Start { id: 1 })));
        assert!(r.poll(20).is_none());
        assert!(r.ack(1));
        assert!(matches!(r.poll(30), Some(Packet::Audio { samples, ended: true, .. }) if samples == [1, 2, 3]));
    }
    #[test]
    fn busy_and_stale_events_cannot_replace_current_session() {
        let mut r = RemoteVoice::default();
        r.begin(2, 0);
        assert!(!r.begin(3, 5));
        r.push(1, &[9]); r.end(1); r.release(1);
        assert!(!r.ack(1));
        assert!(matches!(r.poll(6), Some(Packet::Start { id: 2 })));
        r.ack(2);
        assert!(r.poll(7).is_none());
    }
    #[test]
    fn disconnect_discards_partial_audio_and_reports_failure() {
        let mut r = RemoteVoice::default(); r.begin(1, 0); r.poll(1); r.ack(1);
        r.push(1, &[1, 2]); r.cancel("disconnected");
        assert!(matches!(r.poll(2), Some(Packet::Audio { samples, ended: true, error: Some(_), .. }) if samples.is_empty()));
    }
    #[test]
    fn drain_batches_and_timeout_are_bounded() {
        let mut r = RemoteVoice::default(); r.begin(1, 0); r.poll(1); r.ack(1);
        r.push(1, &vec![7; BATCH_SAMPLES + 1]); r.end(1);
        assert!(matches!(r.poll(2), Some(Packet::Audio { ended: false, .. })));
        assert!(matches!(r.poll(3), Some(Packet::Audio { samples, ended: true, .. }) if samples == [7]));
        r.release(1); assert!(r.begin(2, 4)); assert!(r.begin(3, 30_005));
    }

    #[test]
    fn renderer_reload_releases_stale_session_but_initial_attach_keeps_audio() {
        let mut r = RemoteVoice::default(); r.begin(1, 0); r.push(1, &[3]);
        r.attach("a"); assert!(matches!(r.poll(1), Some(Packet::Start { id: 1 })));
        r.ack(1); r.attach("a");
        assert!(matches!(r.poll(2), Some(Packet::Audio { samples, .. }) if samples == [3]));
        r.attach("b"); assert!(r.poll(3).is_none()); assert!(r.begin(2, 4));
    }

    #[test]
    fn timeout_notifies_an_acknowledged_recorder() {
        let mut r = RemoteVoice::default(); r.begin(1, 0); r.poll(1); r.ack(1);
        assert!(matches!(r.poll(360_001), Some(Packet::Audio { ended: true, error: Some(_), .. })));
    }
}
