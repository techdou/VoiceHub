//! WASAPI 输出运行时：解码后的 PCM 播入所选端点（VB-CABLE Input）。
//!
//! 工作线程持有 IMMDevice/IAudioClient；会话按代际隔离，迟到的样本
//! 直接丢弃。16 kHz 单声道 + 共享模式 autoconvert，由系统混音器完成
//! 格式转换。有界队列防止 BLE 突发把内存打爆。

use std::collections::VecDeque;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use wasapi::{
    AudioClient, AudioRenderClient, DeviceEnumerator, Direction, SampleType, StreamMode,
    WaveFormat,
};

use crate::{AudioEndpoint, PlatformError, Result};

const SOURCE_SAMPLE_RATE: usize = 16_000;
const PREBUFFER_SAMPLES: usize = 480; // 30ms 起播
const MAX_QUEUE_SAMPLES: usize = SOURCE_SAMPLE_RATE * 2; // 2 秒
const MESSAGE_QUEUE_CAPACITY: usize = 64;
const POLL_INTERVAL: Duration = Duration::from_millis(5);
/// 空闲轮询：无会话且队列空时拉长等待（消息到达仍即时唤醒，
/// 超时只是兜底 tick；250ms 让空闲 CPU 唤醒降一个数量级）。
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// 轮询间隔决策（单测覆盖）：忙 = 流式/排空中 / 队列有数据 / 流未停。
fn poll_interval(phase: AudioPhase, queued: usize, sink_started: bool) -> Duration {
    let busy = phase == AudioPhase::Streaming
        || phase == AudioPhase::Draining
        || queued > 0
        || sink_started;
    if busy { POLL_INTERVAL } else { IDLE_POLL_INTERVAL }
}
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioPhase {
    Idle,
    Streaming,
    Draining,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioSnapshot {
    pub phase: AudioPhase,
    pub endpoint_name: Option<String>,
    pub endpoint_id: Option<String>,
    pub generation: u64,
    pub queued_samples: usize,
    pub total_samples_written: u64,
    pub last_error: Option<String>,
}

impl Default for AudioSnapshot {
    fn default() -> Self {
        Self {
            phase: AudioPhase::Idle,
            endpoint_name: None,
            endpoint_id: None,
            generation: 0,
            queued_samples: 0,
            total_samples_written: 0,
            last_error: None,
        }
    }
}

enum AudioMessage {
    ListEndpoints { reply: SyncSender<Result<Vec<AudioEndpoint>>> },
    SelectEndpoint { id: String, reply: SyncSender<Result<AudioSnapshot>> },
    BeginSession { generation: u64, reply: SyncSender<Result<AudioSnapshot>> },
    Samples { generation: u64, samples: Vec<i16> },
    FinishSession { generation: u64, reply: SyncSender<Result<AudioSnapshot>> },
    Shutdown { reply: SyncSender<()> },
}

pub struct AudioRuntime {
    sender: SyncSender<AudioMessage>,
    state: Arc<Mutex<AudioSnapshot>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl AudioRuntime {
    pub fn new() -> Self {
        let (sender, receiver) = sync_channel(MESSAGE_QUEUE_CAPACITY);
        let state = Arc::new(Mutex::new(AudioSnapshot::default()));
        let worker_state = Arc::clone(&state);
        let worker = std::thread::Builder::new()
            .name("sb-wasapi".into())
            .spawn(move || worker_loop(receiver, worker_state))
            .ok();
        Self { sender, state, worker: Mutex::new(worker) }
    }

    pub fn snapshot(&self) -> AudioSnapshot {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn list_endpoints(&self) -> Result<Vec<AudioEndpoint>> {
        self.request(REQUEST_TIMEOUT, |reply| AudioMessage::ListEndpoints { reply })
    }

    pub fn select_endpoint(&self, id: String) -> Result<AudioSnapshot> {
        self.request(REQUEST_TIMEOUT, |reply| AudioMessage::SelectEndpoint { id, reply })
    }

    pub fn begin_session(&self, generation: u64) -> Result<AudioSnapshot> {
        self.request(REQUEST_TIMEOUT, |reply| AudioMessage::BeginSession { generation, reply })
    }

    /// 语音样本（16 kHz 单声道 i16）。队列满 = 溢出丢帧。
    pub fn enqueue_samples(&self, generation: u64, samples: Vec<i16>) -> Result<()> {
        self.sender
            .try_send(AudioMessage::Samples { generation, samples })
            .map_err(|error| match error {
                TrySendError::Full(_) => {
                    PlatformError::Message("音频队列溢出（丢帧）".to_owned())
                }
                TrySendError::Disconnected(_) => {
                    PlatformError::Message("音频工作线程不可用".to_owned())
                }
            })
    }

    pub fn finish_session(&self, generation: u64) -> Result<AudioSnapshot> {
        self.request(DRAIN_TIMEOUT, |reply| AudioMessage::FinishSession { generation, reply })
    }

    pub fn shutdown(self) {
        let (tx, rx) = sync_channel(1);
        let _ = self.sender.send(AudioMessage::Shutdown { reply: tx });
        let _ = rx.recv_timeout(Duration::from_secs(2));
        if let Some(worker) = self.worker.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = worker.join();
        }
    }

    fn request<T>(&self, timeout: Duration, build: impl FnOnce(SyncSender<Result<T>>) -> AudioMessage) -> Result<T> {
        let (tx, rx) = sync_channel(1);
        self.sender
            .try_send(build(tx))
            .map_err(|_| PlatformError::Message("音频请求入队失败".to_owned()))?;
        rx.recv_timeout(timeout)
            .map_err(|_| PlatformError::Message("音频请求超时".to_owned()))?
    }
}

fn worker_loop(receiver: Receiver<AudioMessage>, state: Arc<Mutex<AudioSnapshot>>) {
    let _ = wasapi::initialize_mta();
    let mut sink: Option<AudioSink> = None;
    let mut queue: VecDeque<i16> = VecDeque::with_capacity(MAX_QUEUE_SAMPLES);
    let mut session_generation: u64 = 0;

    loop {
        // 自适应节拍：空闲 250ms、会话中 5ms；消息到达即时唤醒。
        let phase = state.lock().unwrap_or_else(|p| p.into_inner()).phase;
        let sink_started = sink.as_ref().is_some_and(|s| s.started);
        let timeout = poll_interval(phase, queue.len(), sink_started);
        match receiver.recv_timeout(timeout) {
            Ok(AudioMessage::ListEndpoints { reply }) => {
                let _ = reply.send(list_endpoints());
            }
            Ok(AudioMessage::SelectEndpoint { id, reply }) => {
                sink = None;
                queue.clear();
                match AudioSink::open(&id) {
                    Ok(new_sink) => {
                        let mut snapshot = state.lock().unwrap_or_else(|p| p.into_inner());
                        snapshot.endpoint_id = Some(id);
                        snapshot.endpoint_name = Some(new_sink.name.clone());
                        snapshot.phase = AudioPhase::Idle;
                        snapshot.last_error = None;
                        let reply_value = Ok(snapshot.clone());
                        drop(snapshot);
                        sink = Some(new_sink);
                        let _ = reply.send(reply_value);
                    }
                    Err(error) => {
                        fail(&state, format!("打开输出端点失败：{error}"));
                        let _ = reply.send(Err(error));
                    }
                }
            }
            Ok(AudioMessage::BeginSession { generation, reply }) => {
                queue.clear();
                session_generation = generation;
                if let Some(s) = sink.as_mut() {
                    if let Err(error) = s.reset() {
                        fail(&state, format!("重置音频流失败：{error}"));
                    }
                }
                let snapshot = set_phase(&state, AudioPhase::Streaming, generation);
                let _ = reply.send(Ok(snapshot));
            }
            Ok(AudioMessage::Samples { generation, samples }) => {
                if generation == session_generation {
                    for sample in samples {
                        if queue.len() >= MAX_QUEUE_SAMPLES {
                            queue.pop_front(); // 溢出丢最旧帧
                        }
                        queue.push_back(sample);
                    }
                }
            }
            Ok(AudioMessage::FinishSession { generation, reply }) => {
                if generation == session_generation {
                    let drained = drain(&mut queue, sink.as_mut(), &state, generation);
                    let _ = reply.send(drained);
                } else {
                    let snapshot = state.lock().unwrap_or_else(|p| p.into_inner()).clone();
                    let _ = reply.send(Ok(snapshot));
                }
            }
            Ok(AudioMessage::Shutdown { reply }) => {
                if let Some(s) = sink.as_mut() {
                    let _ = s.stop();
                }
                let _ = wasapi::deinitialize();
                let _ = reply.send(());
                return;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                if queue.is_empty() {
                    break;
                }
            }
        }

        // 泵：流式阶段持续写。
        if let Some(s) = sink.as_mut() {
            let phase = state.lock().unwrap_or_else(|p| p.into_inner()).phase;
            if phase == AudioPhase::Streaming {
                if let Err(error) = pump(s, &mut queue, false) {
                    fail(&state, format!("音频写入失败：{error}"));
                }
            }
        }
    }
    let _ = wasapi::deinitialize();
}

fn pump(sink: &mut AudioSink, queue: &mut VecDeque<i16>, draining: bool) -> Result<usize> {
    if !sink.started && queue.len() < PREBUFFER_SAMPLES && !draining {
        return Ok(0);
    }
    let available = sink
        .client
        .get_available_space_in_frames()
        .map_err(|e| PlatformError::Message(format!("{e}")))? as usize;
    let frames = available.min(queue.len());
    if frames == 0 {
        return Ok(0);
    }
    let mut bytes = Vec::with_capacity(frames * 2);
    for _ in 0..frames {
        bytes.extend_from_slice(&queue.pop_front().unwrap().to_le_bytes());
    }
    sink.render_client
        .write_to_device(frames, &bytes, None)
        .map_err(|e| PlatformError::Message(format!("{e}")))?;
    if !sink.started {
        sink.client
            .start_stream()
            .map_err(|e| PlatformError::Message(format!("{e}")))?;
        sink.started = true;
    }
    Ok(frames)
}

fn drain(
    queue: &mut VecDeque<i16>,
    mut sink: Option<&mut AudioSink>,
    state: &Arc<Mutex<AudioSnapshot>>,
    generation: u64,
) -> Result<AudioSnapshot> {
    set_phase(state, AudioPhase::Draining, generation);
    let deadline = std::time::Instant::now() + DRAIN_TIMEOUT;
    while std::time::Instant::now() < deadline {
        if let Some(s) = sink.as_deref_mut() {
            let _ = pump(s, queue, true);
            let drained = queue.is_empty()
                && s
                    .client
                    .get_current_padding()
                    .map(|padding| padding == 0)
                    .unwrap_or(true);
            if drained {
                let _ = s.stop();
                return Ok(set_phase(state, AudioPhase::Idle, generation));
            }
        } else {
            queue.clear();
            return Ok(set_phase(state, AudioPhase::Idle, generation));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    // 超时：硬停并清队列（防粘流）。
    if let Some(s) = sink {
        let _ = s.stop();
    }
    queue.clear();
    Ok(set_phase(state, AudioPhase::Idle, generation))
}

struct AudioSink {
    name: String,
    client: AudioClient,
    render_client: AudioRenderClient,
    started: bool,
}

impl AudioSink {
    fn open(endpoint_id: &str) -> Result<Self> {
        let enumerator =
            DeviceEnumerator::new().map_err(|e| PlatformError::Message(format!("{e}")))?;
        let device = enumerator
            .get_device(endpoint_id)
            .map_err(|e| PlatformError::Message(format!("打开输出端点失败：{e}")))?;
        let name = device
            .get_friendlyname()
            .map_err(|e| PlatformError::Message(format!("{e}")))?;
        let mut client = device
            .get_iaudioclient()
            .map_err(|e| PlatformError::Message(format!("{e}")))?;
        let format = WaveFormat::new(16, 16, &SampleType::Int, SOURCE_SAMPLE_RATE, 1, None);
        let (default_period, _) = client
            .get_device_period()
            .map_err(|e| PlatformError::Message(format!("{e}")))?;
        client
            .initialize_client(
                &format,
                &Direction::Render,
                &StreamMode::PollingShared {
                    autoconvert: true,
                    buffer_duration_hns: default_period,
                },
            )
            .map_err(|e| PlatformError::Message(format!("初始化 16kHz WASAPI 输出失败：{e}")))?;
        let render_client = client
            .get_audiorenderclient()
            .map_err(|e| PlatformError::Message(format!("{e}")))?;
        Ok(Self { name, client, render_client, started: false })
    }

    fn reset(&mut self) -> Result<()> {
        if self.started {
            self.client
                .stop_stream()
                .map_err(|e| PlatformError::Message(format!("{e}")))?;
            self.started = false;
        }
        self.client
            .reset_stream()
            .map_err(|e| PlatformError::Message(format!("{e}")))?;
        Ok(())
    }

    fn stop(&mut self) {
        if self.started {
            let _ = self.client.stop_stream();
            self.started = false;
        }
    }
}

fn list_endpoints() -> Result<Vec<AudioEndpoint>> {
    let enumerator =
        DeviceEnumerator::new().map_err(|e| PlatformError::Message(format!("{e}")))?;
    let collection = enumerator
        .get_device_collection(&Direction::Render)
        .map_err(|e| PlatformError::Message(format!("{e}")))?;
    let mut endpoints = Vec::new();
    for device in &collection {
        let device = device.map_err(|e| PlatformError::Message(format!("{e}")))?;
        let id = device
            .get_id()
            .map_err(|e| PlatformError::Message(format!("{e}")))?;
        let name = device
            .get_friendlyname()
            .map_err(|e| PlatformError::Message(format!("{e}")))?;
        endpoints.push(AudioEndpoint {
            is_virtual_cable_candidate: crate::is_virtual_cable_input_name(&name),
            id,
            name,
        });
    }
    endpoints.sort_by(|l, r| {
        r.is_virtual_cable_candidate
            .cmp(&l.is_virtual_cable_candidate)
            .then_with(|| l.name.to_lowercase().cmp(&r.name.to_lowercase()))
    });
    Ok(endpoints)
}

fn set_phase(state: &Arc<Mutex<AudioSnapshot>>, phase: AudioPhase, generation: u64) -> AudioSnapshot {
    let mut snapshot = state.lock().unwrap_or_else(|p| p.into_inner());
    snapshot.phase = phase;
    snapshot.generation = generation;
    snapshot.queued_samples = 0;
    snapshot.clone()
}

fn fail(state: &Arc<Mutex<AudioSnapshot>>, message: String) {
    let mut snapshot = state.lock().unwrap_or_else(|p| p.into_inner());
    snapshot.phase = AudioPhase::Failed;
    snapshot.last_error = Some(message);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_uses_long_poll_busy_uses_fast_poll() {
        // 完全空闲：长轮询。
        assert_eq!(poll_interval(AudioPhase::Idle, 0, false), IDLE_POLL_INTERVAL);
        assert_eq!(poll_interval(AudioPhase::Failed, 0, false), IDLE_POLL_INTERVAL);
        // 流式 / 排空：快轮询。
        assert_eq!(poll_interval(AudioPhase::Streaming, 0, false), POLL_INTERVAL);
        assert_eq!(poll_interval(AudioPhase::Draining, 0, false), POLL_INTERVAL);
        // 队列有积压：保持快轮询直到排空。
        assert_eq!(poll_interval(AudioPhase::Idle, 1, false), POLL_INTERVAL);
        // 流未停（设备缓冲还有数据）：快轮询。
        assert_eq!(poll_interval(AudioPhase::Idle, 0, true), POLL_INTERVAL);
    }

    #[test]
    fn idle_poll_is_meaningfully_slower() {
        assert!(IDLE_POLL_INTERVAL >= POLL_INTERVAL * 20);
    }
}
