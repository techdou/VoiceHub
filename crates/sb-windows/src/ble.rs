//! WinRT BLE 桥：配对设备枚举、连接、GATT 订阅、ATVV 语音会话。
//!
//! 单工作线程串行驱动状态机（Stalled 状态隔离靠代际计数）；
//! GATT 通知回调（线程池线程）只做转发 + 武装 F5 吞键宽限，
//! 不直接触碰共享可变状态。

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use futures::executor::block_on;
use windows::core::{GUID, HSTRING, IInspectable};
use std::future::IntoFuture;

use windows::Devices::Bluetooth::{BluetoothCacheMode, BluetoothLEDevice};
use windows::Devices::Bluetooth::GenericAttributeProfile::{
    GattCharacteristic, GattCharacteristicProperties, GattCommunicationStatus,
    GattDeviceService, GattValueChangedEventArgs, GattWriteOption,
};
use windows::Devices::Enumeration::DeviceInformation;
use windows::Storage::Streams::{DataReader, DataWriter};

use sb_core::adpcm::ImaAdpcmCodec;
use sb_core::atvv::{AtvvCapabilities, AtvvCommand, AtvvControlEvent, AtvvUuids};
use sb_core::frame::FrameAccumulator;
use sb_core::pcm;
use sb_core::reconnect::ReconnectPolicy;
use sb_core::remote_model::{is_voice_remote_name, RemoteModel};

use crate::key_gate;
use crate::radio;

// ---------- 对外类型 ----------

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionPhase {
    Stopped,
    Scanning,
    Connecting,
    Discovering,
    Ready,
    Reconnecting,
    Failed,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BleSnapshot {
    pub phase: ConnectionPhase,
    pub remote_name: Option<String>,
    pub remote_model: Option<String>,
    pub battery_percent: Option<u8>,
    pub voice_streaming: bool,
    pub session_id: u8,
    pub consecutive_failures: u32,
    pub radio_cycles: u32,
    pub last_error: Option<String>,
}

impl Default for BleSnapshot {
    fn default() -> Self {
        Self {
            phase: ConnectionPhase::Stopped,
            remote_name: None,
            remote_model: None,
            battery_percent: None,
            voice_streaming: false,
            session_id: 0,
            consecutive_failures: 0,
            radio_cycles: 0,
            last_error: None,
        }
    }
}

/// 推给宿主的事件流。
#[derive(Debug, Clone)]
pub enum BleEvent {
    SnapshotChanged,
    /// 语音流开始（Provider 触发 + 音频会话开始）。
    VoiceStarted { session_id: u8 },
    /// 语音流结束（宿主排空后触发 Provider 收尾）。
    VoiceStopped { session_id: u8 },
    /// 解码 + 增益后的 PCM 样本（16 kHz 单声道）。
    Samples { samples: Vec<i16> },
    Battery { percent: u8 },
}

enum Command {
    Connect { device_id: String },
    Disconnect,
    ListPaired { reply: Sender<Vec<PairedRemote>> },
    MicClose,
    ReconnectNow,
    SetGain { gain_db: f64 },
    SetExtendEnabled { enabled: bool },
    Shutdown { reply: Sender<()> },
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairedRemote {
    pub id: String,
    pub name: String,
}

pub struct BleRuntime {
    sender: Sender<Command>,
    state: Arc<Mutex<BleSnapshot>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl BleRuntime {
    pub fn new(events: Sender<BleEvent>) -> Self {
        let (sender, receiver) = channel();
        let state = Arc::new(Mutex::new(BleSnapshot::default()));
        let worker_state = Arc::clone(&state);
        let worker = std::thread::Builder::new()
            .name("sb-ble".into())
            .spawn(move || worker_loop(receiver, events, worker_state))
            .expect("spawn BLE worker");
        Self { sender, state, worker: Mutex::new(Some(worker)) }
    }

    pub fn snapshot(&self) -> BleSnapshot {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn connect(&self, device_id: String) {
        let _ = self.sender.send(Command::Connect { device_id });
    }

    pub fn disconnect(&self) {
        let _ = self.sender.send(Command::Disconnect);
    }

    pub fn reconnect_now(&self) {
        let _ = self.sender.send(Command::ReconnectNow);
    }

    pub fn mic_close(&self) {
        let _ = self.sender.send(Command::MicClose);
    }

    pub fn set_gain(&self, gain_db: f64) {
        let _ = self.sender.send(Command::SetGain { gain_db });
    }

    pub fn set_extend_enabled(&self, enabled: bool) {
        let _ = self.sender.send(Command::SetExtendEnabled { enabled });
    }

    pub fn list_paired(&self) -> Vec<PairedRemote> {
        let (tx, rx) = channel();
        if self.sender.send(Command::ListPaired { reply: tx }).is_ok() {
            rx.recv_timeout(Duration::from_secs(10)).unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    pub fn shutdown(self) {
        let (tx, rx) = channel();
        let _ = self.sender.send(Command::Shutdown { reply: tx });
        let _ = rx.recv_timeout(Duration::from_secs(3));
        if let Some(worker) = self.worker.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = worker.join();
        }
    }
}

// ---------- 工作线程 ----------

struct WorkerContext {
    events: Sender<BleEvent>,
    state: Arc<Mutex<BleSnapshot>>,
    /// 当前连接命令（断连后按它重连）。
    target_device_id: Option<String>,
    should_run: bool,
    reconnect_at: Option<Instant>,
    policy: ReconnectPolicy,
    session: Option<Session>,
    /// 通知回调 → 工作线程（避免在回调里碰 Session）。
    inbox: Receiver<InboxMessage>,
    inbox_sender: Sender<InboxMessage>,
    generation: u64,
    /// 当前增益（新会话建立时应用，防重连后丢失）。
    gain_db: f64,
    /// 实验性续租开关（同 gain_db，跨会话保持，防重连后静默失效）。
    extend_enabled: bool,
}

enum InboxMessage {
    Control { generation: u64, bytes: Vec<u8> },
    Audio { generation: u64, bytes: Vec<u8> },
    ConnectionLost { generation: u64 },
}

struct Session {
    name: String,
    model: RemoteModel,
    device: BluetoothLEDevice,
    service: GattDeviceService,
    transmit: GattCharacteristic,
    audio: GattCharacteristic,
    control: GattCharacteristic,
    capabilities: AtvvCapabilities,
    decoder: ImaAdpcmCodec,
    accumulator: FrameAccumulator,
    pending_sync: Option<(i16, u8)>,
    streaming: bool,
    microphone_opened: bool,
    session_id: u8,
    audio_token: i64,
    control_token: i64,
    connection_token: i64,
    gain_db: f64,
    /// 实验性续租：是否启用、会话起点、上次续租时刻。
    extend_enabled: bool,
    voice_started: Option<Instant>,
    last_extend_at: Option<Instant>,
    /// 续租写失败的退避重试（防僵死链路上 100Hz 重试风暴）。
    extend_retry_at: Option<Instant>,
    extend_failures: u8,
}

impl Session {
    fn write(&self, bytes: &[u8]) -> windows::core::Result<()> {
        let writer = DataWriter::new()?;
        writer.WriteBytes(bytes)?;
        let buffer = writer.DetachBuffer()?;
        let properties = self.transmit.CharacteristicProperties()?;
        let operation = if properties
            .contains(GattCharacteristicProperties::WriteWithoutResponse)
        {
            self.transmit.WriteValueWithOptionAsync(&buffer, GattWriteOption::WriteWithoutResponse)?
        } else {
            self.transmit.WriteValueAsync(&buffer)?
        };
        let result = block_on(operation.into_future())?;
        if result != GattCommunicationStatus::Success {
            return Err(gatt_status_error());
        }
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // 各自注销：audio/control 是 ValueChanged，connection 是
        // ConnectionStatusChanged；CCCD 关闭在物理断连后注定失败，
        // 尽力而为（RemoveXxx 本地句柄必清）。
        let _ = self.audio.RemoveValueChanged(self.audio_token);
        let _ = self.control.RemoveValueChanged(self.control_token);
        let _ = self.device.RemoveConnectionStatusChanged(self.connection_token);
        let _ = self.service.Close();
        let _ = self.device.Close();
    }
}

/// GATT 状态失败 → 通用 E_FAIL（具体状态由调用点日志）。
fn gatt_status_error() -> windows::core::Error {
    windows::core::Error::from_hresult(windows::core::HRESULT(0x8000_4005u32 as i32))
}

fn guid_from_uuid(uuid: &str) -> GUID {
    let hex: String = uuid.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    GUID::from_u128(u128::from_str_radix(&hex, 16).expect("valid uuid"))
}

fn buffer_to_vec(buffer: &windows::Storage::Streams::IBuffer) -> windows::core::Result<Vec<u8>> {
    let reader = DataReader::FromBuffer(buffer)?;
    let mut bytes = vec![0u8; buffer.Length()? as usize];
    reader.ReadBytes(&mut bytes)?;
    Ok(bytes)
}

fn update<F: FnOnce(&mut BleSnapshot)>(state: &Arc<Mutex<BleSnapshot>>, events: &Sender<BleEvent>, f: F) {
    {
        let mut snapshot = state.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut snapshot);
    }
    let _ = events.send(BleEvent::SnapshotChanged);
}

fn worker_loop(receiver: Receiver<Command>, events: Sender<BleEvent>, state: Arc<Mutex<BleSnapshot>>) {
    let (inbox_sender, inbox) = channel();
    let mut ctx = WorkerContext {
        events: events.clone(),
        state: state.clone(),
        target_device_id: None,
        should_run: true,
        reconnect_at: None,
        policy: ReconnectPolicy::default(),
        session: None,
        inbox,
        inbox_sender,
        generation: 0,
        gain_db: 0.0,
        extend_enabled: false,
    };

    while ctx.should_run {
        // 1) 命令
        while let Ok(command) = receiver.try_recv() {
            match command {
                Command::Connect { device_id } => {
                    let target = device_id.clone();
                    ctx.target_device_id = Some(device_id);
                    ctx.policy.reset();
                    close_session(&mut ctx);
                    attempt_connect(&mut ctx, &target);
                }
                Command::Disconnect => {
                    ctx.target_device_id = None;
                    ctx.reconnect_at = None;
                    close_session(&mut ctx);
                    update(&state, &events, |s| {
                        s.phase = ConnectionPhase::Stopped;
                        s.voice_streaming = false;
                        s.last_error = None;
                    });
                }
                Command::ReconnectNow => {
                    ctx.policy.reset();
                    if let Some(device_id) = ctx.target_device_id.clone() {
                        close_session(&mut ctx);
                        attempt_connect(&mut ctx, &device_id);
                    }
                }
                Command::MicClose => {
                    if let Some(session) = ctx.session.as_ref() {
                        let command = AtvvCommand::MicrophoneClose {
                            version: session.capabilities.version,
                            session_id: session.session_id,
                        };
                        if let Some(bytes) = command.encode() {
                            let _ = session.write(&bytes);
                        }
                    }
                }
                Command::SetGain { gain_db } => {
                    let clamped = gain_db.clamp(-24.0, 24.0);
                    ctx.gain_db = clamped;
                    if let Some(session) = ctx.session.as_mut() {
                        session.gain_db = clamped;
                    }
                }
                Command::SetExtendEnabled { enabled } => {
                    ctx.extend_enabled = enabled;
                    if let Some(session) = ctx.session.as_mut() {
                        session.extend_enabled = enabled;
                    }
                }
                Command::ListPaired { reply } => {
                    let _ = reply.send(list_paired_remotes());
                }
                Command::Shutdown { reply } => {
                    ctx.should_run = false;
                    close_session(&mut ctx);
                    let _ = reply.send(());
                }
            }
        }

        // 2) 通知消息
        let mut disconnected = false;
        while let Ok(message) = ctx.inbox.try_recv() {
            match message {
                InboxMessage::Control { generation, bytes } => {
                    if generation == ctx.generation {
                        if let Some(session) = ctx.session.as_mut() {
                            handle_control(session, &ctx.events, &bytes);
                        }
                    }
                }
                InboxMessage::Audio { generation, bytes } => {
                    if generation == ctx.generation {
                        if let Some(session) = ctx.session.as_mut() {
                            process_audio(session, &ctx.events, &bytes);
                        }
                    }
                }
                InboxMessage::ConnectionLost { generation } => {
                    if generation == ctx.generation {
                        disconnected = true;
                    }
                }
            }
        }
        if disconnected {
            // 物理断连：先清掉死会话——重连调度的守卫是 session.is_none()，
            // 不清就永远进不了重连分支；close_session 顺带补发
            // VoiceStopped，让宿主释放 Provider 的按住式触发键。
            close_session(&mut ctx);
            let failures = ctx.policy.consecutive_failures();
            let cycles = ctx
                .state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .radio_cycles;
            if radio::should_cycle(failures, cycles) {
                let result = radio::cycle_bluetooth_radio();
                update(&state, &events, |s| {
                    s.radio_cycles += 1;
                    if let Err(error) = &result {
                        s.last_error = Some(error.clone());
                    }
                });
            }
            schedule_reconnect(&mut ctx);
        }

        // 3) 重连调度
        if ctx.session.is_none() && ctx.target_device_id.is_some() {
            if let Some(when) = ctx.reconnect_at {
                if Instant::now() >= when {
                    let device_id = ctx.target_device_id.clone().unwrap();
                    ctx.reconnect_at = None;
                    attempt_connect(&mut ctx, &device_id);
                }
            }
        }

        // 实验性续租：会话进行中每 40s 尝试 MIC_EXTEND（决策纯函数）。
        if let Some(session) = ctx.session.as_mut() {
            if let Some(bytes) = extend_command_if_due(session) {
                match session.write(&bytes) {
                    Ok(()) => {
                        session.last_extend_at = Some(Instant::now());
                        session.extend_retry_at = None;
                        session.extend_failures = 0;
                        log::info!(
                            "ATVV MIC_EXTEND sent session={} (experimental)",
                            session.session_id
                        );
                    }
                    Err(error) => {
                        log::warn!("ATVV MIC_EXTEND write failed: {error}");
                        // 失败不能热循环（10ms 节拍 = 100Hz GATT 写风暴），
                        // 也不能等满下个 40s 周期（会错过 60s 固件租期墙）：
                        // 5s 退避重试，连失败 3 次放弃本轮。
                        session.extend_failures += 1;
                        if session.extend_failures >= 3 {
                            session.extend_failures = 0;
                            session.extend_retry_at = None;
                            session.last_extend_at = Some(Instant::now());
                            log::warn!("ATVV MIC_EXTEND 连续失败 3 次，本轮放弃");
                        } else {
                            session.extend_retry_at =
                                Some(Instant::now() + Duration::from_secs(5));
                        }
                    }
                }
            }
        }

        // 节拍：命令/通知均非阻塞收取，必须让出 CPU。
        std::thread::sleep(Duration::from_millis(10));
    }
    close_session(&mut ctx);
}

fn close_session(ctx: &mut WorkerContext) {
    // 流中关会话必须补发 VoiceStopped：宿主要靠它释放 Provider 的
    // 按住式触发键（右 Alt/右 Ctrl），漏发 = 修饰键系统级卡死。
    let interrupted = ctx
        .session
        .as_ref()
        .filter(|s| s.streaming)
        .map(|s| s.session_id);
    ctx.session.take();
    key_gate::set_session_active(false);
    if let Some(session_id) = interrupted {
        update(&ctx.state, &ctx.events, |s| {
            s.voice_streaming = false;
        });
        let _ = ctx.events.send(BleEvent::VoiceStopped { session_id });
    }
}

fn schedule_reconnect(ctx: &mut WorkerContext) {
    let Some(_) = ctx.target_device_id else { return };
    let jitter = fastrand_like();
    let delay = ctx.policy.next_delay(jitter);
    ctx.reconnect_at = Some(Instant::now() + delay);
    update(&ctx.state, &ctx.events, |s| {
        s.phase = ConnectionPhase::Reconnecting;
        s.voice_streaming = false;
        s.consecutive_failures = s.consecutive_failures.saturating_add(1);
    });
}

/// 简易 jitter（不引入 rand 依赖）。
fn fastrand_like() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    (nanos % 1000) as f64 / 1000.0
}

fn attempt_connect(ctx: &mut WorkerContext, device_id: &str) {
    ctx.generation += 1;
    let generation = ctx.generation;
    update(&ctx.state, &ctx.events, |s| {
        s.phase = ConnectionPhase::Connecting;
        s.last_error = None;
    });

    match connect_session(device_id, ctx.inbox_sender.clone(), generation, ctx.gain_db, ctx.extend_enabled) {
        Ok(session) => {
            let name = session.name.clone();
            let model = session.model;
            let model_name = model.display_name().to_string();
            update(&ctx.state, &ctx.events, |s| {
                s.phase = ConnectionPhase::Ready;
                s.remote_name = Some(name);
                s.remote_model = Some(model_name);
                s.consecutive_failures = 0;
                s.radio_cycles = 0;
            });
            // 电池（尽力而为）。
            if let Some(percent) = read_battery(&session.device) {
                update(&ctx.state, &ctx.events, |s| {
                    s.battery_percent = Some(percent);
                });
                let _ = ctx.events.send(BleEvent::Battery { percent });
            }
            ctx.session = Some(session);
        }
        Err(error) => {
            let message = format!("{error}");
            log::warn!("BLE connect failed: {message}");
            update(&ctx.state, &ctx.events, |s| {
                s.phase = ConnectionPhase::Failed;
                s.last_error = Some(message);
            });
            let jitter = fastrand_like();
            let delay = ctx.policy.next_delay(jitter);
            ctx.reconnect_at = Some(Instant::now() + delay);
            update(&ctx.state, &ctx.events, |s| {
                s.phase = ConnectionPhase::Reconnecting;
                s.consecutive_failures = s.consecutive_failures.saturating_add(1);
            });
        }
    }
}

fn connect_session(
    device_id: &str,
    inbox: Sender<InboxMessage>,
    generation: u64,
    gain_db: f64,
    extend_enabled: bool,
) -> windows::core::Result<Session> {
    let device = block_on(BluetoothLEDevice::FromIdAsync(&HSTRING::from(device_id))?.into_future())?;
    let name = device.Name()?.to_string();
    if name.is_empty() && !device_id.is_empty() {
        log::info!("BLE device name empty; continuing by id");
    }
    let model = remote_model_from_name(&name)
        .or_else(|| read_remote_model(&device))
        .unwrap_or(RemoteModel::Unknown);

    let service = find_service(&device, &AtvvUuids::SERVICE)?;
    let transmit = find_characteristic(&service, &AtvvUuids::TRANSMIT)?;
    let audio = find_characteristic(&service, &AtvvUuids::AUDIO)?;
    let control = find_characteristic(&service, &AtvvUuids::CONTROL)?;

    // 订阅 audio + control（token 分别保存，Drop 时各自注销）。
    let audio_token = subscribe(&audio, inbox.clone(), generation, false)?;
    let control_token = subscribe(&control, inbox.clone(), generation, true)?;

    // 连接状态监视。
    let status_sender = inbox.clone();
    let handler = windows::Foundation::TypedEventHandler::<
        BluetoothLEDevice,
        IInspectable,
    >::new(move |device, _| {
        if let Some(device) = device.as_ref() {
            if let Ok(status) = device.ConnectionStatus() {
                if status == windows::Devices::Bluetooth::BluetoothConnectionStatus::Disconnected {
                    let _ = status_sender.send(InboxMessage::ConnectionLost { generation });
                }
            }
        }
        Ok(())
    });
    let connection_token = device.ConnectionStatusChanged(&handler)?;

    // 默认能力（v1 / 16k / 120 帧长）：真正的能力在控制通知里刷新。
    let mut session = Session {
        name,
        model,
        device,
        service,
        transmit,
        audio,
        control,
        capabilities: default_capabilities(),
        decoder: ImaAdpcmCodec::new(),
        accumulator: FrameAccumulator::new(),
        pending_sync: None,
        streaming: false,
        microphone_opened: false,
        session_id: 0,
        audio_token,
        control_token,
        connection_token,
        gain_db: 0.0,
        extend_enabled,
        voice_started: None,
        last_extend_at: None,
        extend_retry_at: None,
        extend_failures: 0,
    };
    if session.model.adpcm_low_nibble_first() {
        session.decoder.set_low_nibble_first(true);
    }
    session.gain_db = gain_db.clamp(-24.0, 24.0);
    session.write(&AtvvCommand::GetCapabilitiesV10.encode().expect("always encodable"))?;
    Ok(session)
}

fn default_capabilities() -> AtvvCapabilities {
    AtvvCapabilities {
        version: 0x0100,
        codecs: 0x02,
        interaction: 0x03,
        frame_size: 120,
        selected_codec: 0x02,
        sample_rate: 16_000,
    }
}

fn remote_model_from_name(name: &str) -> Option<RemoteModel> {
    let model = RemoteModel::identify(name);
    (model != RemoteModel::Unknown).then_some(model)
}

fn read_remote_model(device: &BluetoothLEDevice) -> Option<RemoteModel> {
    let services = block_on(device.GetGattServicesAsync().ok()?.into_future()).ok()?;
    if services.Status().ok()? != GattCommunicationStatus::Success {
        return None;
    }
    let services = services.Services().ok()?;
    for index in 0..services.Size().ok()? {
        let service = services.GetAt(index).ok()?;
        if service.Uuid().ok()? == guid_from_uuid("0000180A-0000-1000-8000-00805F9B34FB") {
            let chars = block_on(service.GetCharacteristicsAsync().ok()?.into_future()).ok()?;
            if chars.Status().ok()? != GattCommunicationStatus::Success {
                return None;
            }
            let chars = chars.Characteristics().ok()?;
            for char_index in 0..chars.Size().ok()? {
                let characteristic = chars.GetAt(char_index).ok()?;
                if characteristic.Uuid().ok()? == guid_from_uuid("00002A24-0000-1000-8000-00805F9B34FB") {
                    let result = block_on(characteristic.ReadValueAsync().ok()?.into_future()).ok()?;
                    if result.Status().ok()? != GattCommunicationStatus::Success {
                        return None;
                    }
                    let bytes = buffer_to_vec(&result.Value().ok()?).ok()?;
                    let text = String::from_utf8_lossy(&bytes).to_string();
                    return Some(RemoteModel::identify(&text));
                }
            }
        }
    }
    None
}

fn read_battery(device: &BluetoothLEDevice) -> Option<u8> {
    let services = block_on(device.GetGattServicesAsync().ok()?.into_future()).ok()?;
    if services.Status().ok()? != GattCommunicationStatus::Success {
        return None;
    }
    let services = services.Services().ok()?;
    for index in 0..services.Size().ok()? {
        let service = services.GetAt(index).ok()?;
        if service.Uuid().ok()? == guid_from_uuid("0000180F-0000-1000-8000-00805F9B34FB") {
            let chars = block_on(service.GetCharacteristicsAsync().ok()?.into_future()).ok()?;
            if chars.Status().ok()? != GattCommunicationStatus::Success {
                return None;
            }
            let chars = chars.Characteristics().ok()?;
            for char_index in 0..chars.Size().ok()? {
                let characteristic = chars.GetAt(char_index).ok()?;
                if characteristic.Uuid().ok()? == guid_from_uuid("00002A19-0000-1000-8000-00805F9B34FB") {
                    let result = block_on(characteristic.ReadValueAsync().ok()?.into_future()).ok()?;
                    if result.Status().ok()? != GattCommunicationStatus::Success {
                        return None;
                    }
                    let bytes = buffer_to_vec(&result.Value().ok()?).ok()?;
                    return bytes.first().copied();
                }
            }
        }
    }
    None
}

fn find_service(device: &BluetoothLEDevice, uuid: &str) -> windows::core::Result<GattDeviceService> {
    let result = block_on(device.GetGattServicesForUuidWithCacheModeAsync(
        guid_from_uuid(uuid),
        BluetoothCacheMode::Uncached,
    )?.into_future())?;
    if result.Status()? != GattCommunicationStatus::Success {
        return Err(gatt_status_error());
    }
    let services = result.Services()?;
    if services.Size()? != 1 {
        return Err(gatt_status_error());
    }
    services.GetAt(0)
}

fn find_characteristic(
    service: &GattDeviceService,
    uuid: &str,
) -> windows::core::Result<GattCharacteristic> {
    let result = block_on(service.GetCharacteristicsForUuidAsync(guid_from_uuid(uuid))?.into_future())?;
    if result.Status()? != GattCommunicationStatus::Success {
        return Err(gatt_status_error());
    }
    let characteristics = result.Characteristics()?;
    if characteristics.Size()? != 1 {
        return Err(gatt_status_error());
    }
    characteristics.GetAt(0)
}

fn subscribe(
    characteristic: &GattCharacteristic,
    inbox: Sender<InboxMessage>,
    generation: u64,
    is_control: bool,
) -> windows::core::Result<i64> {
    use windows::Devices::Bluetooth::GenericAttributeProfile::GattClientCharacteristicConfigurationDescriptorValue;
    use windows::Foundation::TypedEventHandler;
    let sender = inbox;
    let handler = TypedEventHandler::<GattCharacteristic, GattValueChangedEventArgs>::new(
        move |_, args| {
            let Some(args) = args.as_ref() else { return Ok(()) };
            let Ok(value) = args.CharacteristicValue() else { return Ok(()) };
            let Ok(bytes) = buffer_to_vec(&value) else { return Ok(()) };
            if is_control {
                // GATT 回调线程立即武装 F5 吞键宽限（参考实现实证：
                // 工作线程可能被后台节流拖 ~120ms，等不起）。
                key_gate::arm_grace();
                let _ = sender.send(InboxMessage::Control { generation, bytes });
            } else {
                let _ = sender.send(InboxMessage::Audio { generation, bytes });
            }
            Ok(())
        },
    );
    let token = characteristic.ValueChanged(&handler)?;
    let status = block_on(characteristic.WriteClientCharacteristicConfigurationDescriptorAsync(
        GattClientCharacteristicConfigurationDescriptorValue::Notify,
    )?.into_future())?;
    if status != GattCommunicationStatus::Success {
        let _ = characteristic.RemoveValueChanged(token);
        return Err(gatt_status_error());
    }
    Ok(token)
}

/// 续租间隔：40s（在 60s 固件租期内提前续）。
pub const EXTEND_INTERVAL: Duration = Duration::from_secs(40);

/// 纯决策：此刻是否应发送 MIC_EXTEND（单测覆盖）。
/// 条件：实验开启 && 流中 && 已持续 ≥40s && 距上次续租（若有）≥40s。
pub fn should_extend(
    streaming: bool,
    extend_enabled: bool,
    held_for: Option<Duration>,
    since_last_extend: Option<Duration>,
) -> bool {
    if !streaming || !extend_enabled {
        return false;
    }
    let Some(held) = held_for else { return false };
    if held < EXTEND_INTERVAL {
        return false;
    }
    match since_last_extend {
        None => true,
        Some(elapsed) => elapsed >= EXTEND_INTERVAL,
    }
}

/// 生成续租命令（会话状态 → 命令字节）。
fn extend_command_if_due(session: &Session) -> Option<Vec<u8>> {
    if !session.streaming || !session.extend_enabled {
        return None;
    }
    // 有待退避的重试时以重试时刻为准（覆盖常规 40s 节拍，防热循环）。
    if let Some(retry_at) = session.extend_retry_at {
        if Instant::now() < retry_at {
            return None;
        }
    } else {
        let held = session
            .voice_started
            .map(|started| started.elapsed())
            .filter(|held| !held.is_zero());
        let since = session
            .last_extend_at
            .map(|at| at.elapsed())
            .filter(|since| !since.is_zero());
        if !should_extend(session.streaming, session.extend_enabled, held, since) {
            return None;
        }
    }
    AtvvCommand::MicrophoneExtend {
        version: session.capabilities.version,
        session_id: session.session_id,
    }
    .encode()
}

fn list_paired_remotes() -> Vec<PairedRemote> {
    let selector = match BluetoothLEDevice::GetDeviceSelectorFromPairingState(true) {
        Ok(selector) => selector,
        Err(_) => return Vec::new(),
    };
    let operation = match DeviceInformation::FindAllAsyncAqsFilter(&selector) {
        Ok(operation) => operation,
        Err(_) => return Vec::new(),
    };
    let devices = match block_on(operation.into_future()) {
        Ok(devices) => devices,
        Err(_) => return Vec::new(),
    };
    let mut remotes = Vec::new();
    let Ok(size) = devices.Size() else { return remotes };
    for index in 0..size {
        if let Ok(device) = devices.GetAt(index) {
            let name = device.Name().map(|n| n.to_string()).unwrap_or_default();
            if is_voice_remote_name(Some(&name)) {
                let id = device.Id().map(|i| i.to_string()).unwrap_or_default();
                if !id.is_empty() {
                    remotes.push(PairedRemote { id, name });
                }
            }
        }
    }
    remotes
}

// ---------- 会话内协议处理 ----------

fn handle_control(session: &mut Session, events: &Sender<BleEvent>, bytes: &[u8]) {
    let Ok(event) = AtvvControlEvent::parse(bytes) else { return };
    match event {
        AtvvControlEvent::Capabilities(capabilities) => {
            log::info!(
                "ATVV caps version=0x{:04X} codec=0x{:02X} frame={} rate={}",
                capabilities.version,
                capabilities.selected_codec,
                capabilities.frame_size,
                capabilities.sample_rate
            );
            session.capabilities = capabilities;
        }
        AtvvControlEvent::MicrophoneOpenRequested => {
            let command = AtvvCommand::MicrophoneOpen {
                version: session.capabilities.version,
                codec: session.capabilities.selected_codec,
            };
            if let Some(bytes) = command.encode() {
                if session.write(&bytes).is_ok() {
                    session.microphone_opened = true;
                }
            }
        }
        AtvvControlEvent::StreamStarted { codec, session_id, .. } => {
            if let Some(codec) = codec {
                if codec != session.capabilities.selected_codec {
                    // 流实际编码以 0x04 携带为准。
                    session.capabilities.selected_codec = codec;
                    session.capabilities.sample_rate = if codec == 0x02 { 16_000 } else { 8_000 };
                }
            }
            if !session.capabilities.supports_16k_audio() {
                log::warn!("remote advertised non-16k codec; ignoring stream");
                return;
            }
            session.accumulator.reset();
            session.decoder.reset();
            session.pending_sync = None;
            if !session.streaming {
                session.streaming = true;
                session.session_id = session_id;
                session.voice_started = Some(Instant::now());
                session.last_extend_at = None;
                key_gate::set_session_active(true);
                let _ = events.send(BleEvent::VoiceStarted { session_id });
            }
        }
        AtvvControlEvent::StreamStopped => {
            if session.streaming {
                session.streaming = false;
                session.voice_started = None;
                session.last_extend_at = None;
                session.microphone_opened = false;
                key_gate::set_session_active(false);
                let session_id = session.session_id;
                let _ = events.send(BleEvent::VoiceStopped { session_id });
            }
        }
        AtvvControlEvent::DecoderSync { predictor, step_index } => {
            session.accumulator.reset();
            session.pending_sync = Some((predictor, step_index));
        }
        AtvvControlEvent::Unknown { opcode } => {
            log::debug!("ATVV unknown control opcode 0x{opcode:02X}");
        }
    }
}

/// 处理音频通知（由工作循环直接调用）。
fn process_audio(session: &mut Session, events: &Sender<BleEvent>, bytes: &[u8]) {
    if !session.streaming {
        // 隐式开流竞态：0x04 晚于首包音频时按音频到达开流。
        session.accumulator.reset();
        session.decoder.reset();
        session.streaming = true;
        session.voice_started = Some(Instant::now());
        session.last_extend_at = None;
        key_gate::set_session_active(true);
        let _ = events.send(BleEvent::VoiceStarted { session_id: session.session_id });
    }
    let frame_size = session.capabilities.frame_size;
    let frames = session.accumulator.append(bytes, frame_size);
    for frame in frames {
        if let Some((predictor, step_index)) = session.pending_sync.take() {
            session.decoder.reset_with(predictor, step_index);
        }
        let decoded = session.decoder.decode(&frame);
        let processed = pcm::postprocess(&decoded, session.gain_db);
        let _ = events.send(BleEvent::Samples { samples: processed });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_conversion_roundtrip() {
        let guid = guid_from_uuid("AB5E0001-5A21-4F05-BC7D-AF01F617B664");
        assert_eq!(guid.data1, 0xAB5E_0001);
        assert_eq!(guid.data2, 0x5A21);
        assert_eq!(guid.data3, 0x4F05);
    }

    #[test]
    fn extend_policy_fires_after_40s_and_repeats_periodically() {
        use std::time::Duration as D;
        let forty = EXTEND_INTERVAL;
        // 未开启 / 未流中 / 时长不足：不发。
        assert!(!should_extend(true, false, Some(forty), None));
        assert!(!should_extend(false, true, Some(forty), None));
        assert!(!should_extend(true, true, Some(forty - D::from_secs(1)), None));
        // 40s 首次触发；刚续过未满 40s 不再发；满 40s 重发。
        assert!(should_extend(true, true, Some(forty), None));
        assert!(!should_extend(
            true,
            true,
            Some(forty + D::from_secs(5)),
            Some(D::from_secs(5))
        ));
        assert!(should_extend(true, true, Some(forty * 2), Some(forty)));
        // 无时长信息（未开流）不触发。
        assert!(!should_extend(true, true, None, None));
    }

    #[test]
    fn extend_encodes_for_v1_firmware() {
        let command = AtvvCommand::MicrophoneExtend { version: 0x0100, session_id: 7 };
        assert_eq!(command.encode(), Some(vec![0x0E, 7]));
    }

    #[test]
    fn paired_name_filter_is_vendor_agnostic() {
        assert!(is_voice_remote_name(Some("Mi RC Pro")));
        assert!(!is_voice_remote_name(Some("WH-1000XM5")));
    }
}
