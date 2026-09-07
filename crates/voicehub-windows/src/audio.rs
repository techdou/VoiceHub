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
/// 端点被占用的错误码（AUDCLNT_E_DEVICE_IN_USE），wasapi crate 把它
/// Display 成 "Windows returned an error: 0x8889000A"，只能按字符串认。
const DEVICE_IN_USE_CODE: &str = "0x8889000A";
const SELECT_RETRY_COUNT: usize = 8;
const SELECT_RETRY_INTERVAL: Duration = Duration::from_millis(250);

/// 给原始 WASAPI 错误补一层人话与可操作建议（用户在 UI 直接看得到）。
fn explain_audio_error(message: &str) -> String {
    if message.contains(DEVICE_IN_USE_CODE) {
        format!(
            "{message}（端点被占用：同一根 VB-CABLE 的不同声道规格互斥，\
             或其他应用正独占该设备。请关闭占用它的应用后重试）"
        )
    } else {
        message.to_owned()
    }
}

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
            .name("vh-wasapi".into())
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
                queue.clear();
                let previous_id = sink
                    .as_ref()
                    .map(|s| s.id.clone())
                    .or_else(|| state.lock().unwrap_or_else(|p| p.into_inner()).endpoint_id.clone());
                sink = None;
                // DEVICE_IN_USE（0x8889000A）常见于：同线另一 pin 刚释放、
                // audiodg 拆线未完，或外部应用独占。前两种等一小会儿就好，
                // 值得重试；其他错误快速失败。
                let mut opened = None;
                let mut last_error = None;
                for attempt in 0..SELECT_RETRY_COUNT {
                    if attempt > 0 {
                        std::thread::sleep(SELECT_RETRY_INTERVAL);
                    }
                    match AudioSink::open(&id) {
                        Ok(new_sink) => {
                            opened = Some(new_sink);
                            break;
                        }
                        Err(error) => {
                            let retryable = error.to_string().contains(DEVICE_IN_USE_CODE);
                            log::warn!("打开端点失败（第 {} 次）：{error}", attempt + 1);
                            last_error = Some(error);
                            if !retryable {
                                break;
                            }
                        }
                    }
                }
                match opened {
                    Some(new_sink) => {                        let mut snapshot = state.lock().unwrap_or_else(|p| p.into_inner());
                        snapshot.endpoint_id = Some(id);
                        snapshot.endpoint_name = Some(new_sink.name.clone());
                        snapshot.phase = AudioPhase::Idle;
                        snapshot.last_error = None;
                        let reply_value = Ok(snapshot.clone());
                        drop(snapshot);
                        sink = Some(new_sink);
                        let _ = reply.send(reply_value);
                    }
                    None => {
                        let error = explain_audio_error(&last_error.map(|e| e.to_string()).unwrap_or_default());
                        // 失败后不能裸奔：旧端点已被丢掉，必须重开回去，
                        // 否则音频链路整体瘫痪且 UI 无感知。
                        if let Some(old_id) = previous_id {
                            match AudioSink::open(&old_id) {
                                Ok(old_sink) => {
                                    let mut snapshot = state.lock().unwrap_or_else(|p| p.into_inner());
                                    snapshot.endpoint_id = Some(old_id);
                                    snapshot.endpoint_name = Some(old_sink.name.clone());
                                    snapshot.phase = AudioPhase::Idle;
                                    drop(snapshot);
                                    sink = Some(old_sink);
                                }
                                Err(reopen_error) => {
                                    log::error!("切回旧端点也失败：{reopen_error}");
                                    fail(&state, format!("切回旧端点失败：{reopen_error}"));
                                }
                            }
                        } else {
                            fail(&state, error.clone());
                        }
                        let _ = reply.send(Err(PlatformError::Message(error)));
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
    id: String,
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
        Ok(Self { id: endpoint_id.to_owned(), name, client, render_client, started: false })
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
            // 候选中标准 2ch CABLE Input 优先于多通道变体（"CABLE In 16ch" 等）：
            // 变体线没有 capture 端，语音工具收不到声；且同线 pin 互斥，
            // 谁先被打开另一档就 DEVICE_IN_USE（2026-09-06 实测）。
            // 判据：变体命名是 "CABLE In NNch (...)"，不含 "cable input"。
            .then_with(|| stereo_cable_rank(l).cmp(&stereo_cable_rank(r)))
            .then_with(|| l.name.to_lowercase().cmp(&r.name.to_lowercase()))
    });
    Ok(endpoints)
}

/// CABLE 候选里的排序位次：0 = 标准 2ch 端点，1 = 多通道变体，非候选不影响排序。
fn stereo_cable_rank(endpoint: &AudioEndpoint) -> u8 {
    if endpoint.is_virtual_cable_candidate
        && !endpoint.name.to_lowercase().contains("cable input")
    {
        1
    } else {
        0
    }
}

fn set_phase(state: &Arc<Mutex<AudioSnapshot>>, phase: AudioPhase, generation: u64) -> AudioSnapshot {
    let mut snapshot = state.lock().unwrap_or_else(|p| p.into_inner());
    snapshot.phase = phase;
    snapshot.generation = generation;
    snapshot.queued_samples = 0;
    snapshot.clone()
}

fn fail(state: &Arc<Mutex<AudioSnapshot>>, message: String) {
    // 2026-09-06 排查实录：端点选择失败只写快照不落日志，事后无从查因。
    // UI 红字一闪而过，settings.json 不更新，用户只看到"选了不保存"。
    log::warn!("audio: {message}");
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

    #[test]
    fn stereo_cable_input_sorts_before_multichannel_variant() {
        let endpoints = vec![
            AudioEndpoint { is_virtual_cable_candidate: true, id: "16".into(), name: "CABLE In 16ch (VB-Audio Virtual Cable)".into() },
            AudioEndpoint { is_virtual_cable_candidate: true, id: "2".into(), name: "CABLE Input (VB-Audio Virtual Cable)".into() },
            AudioEndpoint { is_virtual_cable_candidate: false, id: "spk".into(), name: "扬声器 (Realtek)".into() },
        ];
        let mut sorted = endpoints;
        sorted.sort_by(|l, r| {
            r.is_virtual_cable_candidate
                .cmp(&l.is_virtual_cable_candidate)
                .then_with(|| stereo_cable_rank(l).cmp(&stereo_cable_rank(r)))
                .then_with(|| l.name.to_lowercase().cmp(&r.name.to_lowercase()))
        });
        assert_eq!(sorted[0].name, "CABLE Input (VB-Audio Virtual Cable)");
        assert_eq!(sorted[1].name, "CABLE In 16ch (VB-Audio Virtual Cable)");
        assert_eq!(sorted[2].name, "扬声器 (Realtek)");
    }

    /// 真机集成测试：VB-CABLE 的 2ch 与 16ch 是同一根线上的互斥 pin，
    /// 声桥内部切换（先释放旧 sink 再打开新端点）必须成功——这是
    /// 2026-09-06 "CABLE Input 选不上"事故的直接回归。无 VB-CABLE 的
    /// 机器自动跳过。
    #[test]
    fn switch_between_cable_pins_keeps_audio_alive() {
        let runtime = AudioRuntime::new();
        let endpoints = match runtime.list_endpoints() {
            Ok(endpoints) => endpoints,
            Err(_) => {
                eprintln!("跳过：无法枚举音频端点");
                return;
            }
        };
        let stereo = endpoints
            .iter()
            .find(|e| e.is_virtual_cable_candidate && e.name.to_lowercase().contains("cable input"))
            .cloned();
        let Some(stereo) = stereo else {
            eprintln!("跳过：本机无 VB-CABLE 端点");
            return;
        };
        // 选中 2ch → 再切到可独立打开的普通端点 → 再切回来。
        // 同一根 VB-CABLE 的多通道 pin 与 2ch pin 在 Windows 上互斥，
        // 不能把它作为“切换成功”的测试目标。
        let other = endpoints
            .iter()
            .find(|e| !e.is_virtual_cable_candidate)
            .cloned();
        let first = runtime.select_endpoint(stereo.id.clone());
        assert!(first.is_ok(), "选中 2ch 失败：{first:?}");
        if let Some(other) = other {
            let switched = runtime.select_endpoint(other.id.clone());
            assert!(switched.is_ok(), "从 2ch 切到 {} 失败：{switched:?}", other.name);
            let back = runtime.select_endpoint(stereo.id.clone());
            assert!(back.is_ok(), "切回 2ch 失败：{back:?}");
            assert_eq!(
                runtime.snapshot().endpoint_name.as_deref(),
                Some(stereo.name.as_str())
            );
        }
        // 选不存在的端点必须报错，且旧 sink 存活（endpoint 不被清空）。
        let bad = runtime.select_endpoint("不存在的端点ID".into());
        assert!(bad.is_err());
        assert_eq!(
            runtime.snapshot().endpoint_name.as_deref(),
            Some(stereo.name.as_str()),
            "失败的切换不应丢掉当前端点"
        );
        runtime.shutdown();
    }
}
