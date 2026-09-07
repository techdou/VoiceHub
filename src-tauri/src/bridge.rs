//! 桥接编排器：BLE / HID / 音频 / Provider / 统计 全链路调度。
//!
//! 事件拓扑：
//! - HID 线程 → InternalEvent::Hid → usage 差分 → 手势识别 → 动作分发
//! - BLE 工作线程 → InternalEvent::Ble → 语音管线（音频泵 + Provider 触发）
//! - 电源线程 → InternalEvent::Power → 唤醒重连
//! - 手势 tick 线程（15ms）→ 单击窗口/长按判定推进
//! 可变共享状态集中在 `BridgeInner`（单把互斥锁；事件频率下无争用压力），
//! UI 状态经 AppHandle emit 推给前端。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use voicehub_core::actions::ButtonAction;
use voicehub_core::buttons::RemoteButton;
use voicehub_core::gesture::{Gesture, GestureRecognizer};
use voicehub_core::provider::ProviderTrigger;
use voicehub_core::settings::{AppSettings, VoiceSessionRecord};
use voicehub_core::statistics::{UsageEvent, UsageStatistics};

use crate::store::Store;
use voicehub_windows::audio::AudioRuntime;
use voicehub_windows::ble::{BleEvent, BleRuntime};
use voicehub_windows::key_gate;
use voicehub_windows::raw_input::{spawn_hid_monitor, HidEvent, HidInput, UsageTracker};

enum InternalEvent {
    Hid(HidInput),
    Ble(BleEvent),
    Power(voicehub_windows::power::PowerEvent),
    /// 手势 tick（驱动单击窗口/长按判定）。
    Tick,
}

#[derive(Debug, Clone, serde::Serialize)]
// 前端按 PascalCase 判别（types.ts / App.vue / 各页面 switch），
// camelCase 会把 tag 转成 "bleState" 导致事件永远匹配不上——勿改。
#[serde(rename_all = "PascalCase", tag = "type")]
pub enum UiEvent {
    BleState { snapshot: voicehub_windows::ble::BleSnapshot },
    VoiceState { recording: bool, level: f32 },
    Battery { percent: u8 },
    ActionReceipt { button: String, gesture: String, action: String, ok: bool },
    /// 物理按键按下 / 释放沿（画布实时高亮）。
    ButtonActivity { button: String, pressed: bool },
    ShowSettings,
    AudioEndpointChanged { name: String },
}

struct BridgeInner {
    settings: AppSettings,
    statistics: UsageStatistics,
    gesture: GestureRecognizer,
    usage_tracker: UsageTracker,
}

pub struct Bridge {
    inner: Mutex<BridgeInner>,
    pub store: Arc<Store>,
    pub app: AppHandle,
    pub ble: BleRuntime,
    pub audio: AudioRuntime,
    voice_generation: AtomicU64,
    voice_started_at_ms: AtomicU64,
    voice_active: AtomicBool,
    voice_finishing: AtomicBool,
    voice_lifecycle: Mutex<()>,
    level_packet_count: AtomicU64,
    pub remote_voice: Mutex<crate::remote_voice::RemoteVoice>,
    voice_provider: Mutex<Option<(u8, voicehub_core::provider::ProviderConfig)>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Bridge {
    pub fn start(app: AppHandle, store: Arc<Store>, settings: AppSettings) -> Arc<Self> {
        let statistics = store.load_statistics();
        let (tx, rx) = channel::<InternalEvent>();

        // BLE 事件转发。
        let (ble_tx, ble_rx) = channel::<BleEvent>();
        {
            let forward = tx.clone();
            std::thread::Builder::new()
                .name("vh-ble-forward".into())
                .spawn(move || {
                    while let Ok(event) = ble_rx.recv() {
                        let _ = forward.send(InternalEvent::Ble(event));
                    }
                })
                .expect("spawn ble forward");
        }
        let ble_runtime = BleRuntime::new(ble_tx);
        let audio_runtime = AudioRuntime::new();

        let bridge = Arc::new(Self {
            inner: Mutex::new(BridgeInner {
                settings,
                statistics,
                gesture: GestureRecognizer::new(),
                usage_tracker: UsageTracker::default(),
            }),
            store: store.clone(),
            app: app.clone(),
            ble: ble_runtime,
            audio: audio_runtime,
            voice_generation: AtomicU64::new(0),
            voice_started_at_ms: AtomicU64::new(0),
            voice_active: AtomicBool::new(false),
            voice_finishing: AtomicBool::new(false),
            voice_lifecycle: Mutex::new(()),
            level_packet_count: AtomicU64::new(0),
            remote_voice: Mutex::new(Default::default()),
            voice_provider: Mutex::new(None),
        });

        // HID 监视。
        {
            let (hid_tx, hid_rx) = channel::<HidInput>();
            let _ = spawn_hid_monitor(hid_tx);
            let forward = tx.clone();
            std::thread::Builder::new()
                .name("vh-hid-forward".into())
                .spawn(move || {
                    while let Ok(event) = hid_rx.recv() {
                        let _ = forward.send(InternalEvent::Hid(event));
                    }
                })
                .expect("spawn hid forward");
        }

        // 电源通知。
        {
            let (power_tx, power_rx) = channel::<voicehub_windows::power::PowerEvent>();
            let _ = voicehub_windows::power::spawn_power_monitor(power_tx);
            let forward = tx.clone();
            std::thread::Builder::new()
                .name("vh-power-forward".into())
                .spawn(move || {
                    while let Ok(event) = power_rx.recv() {
                        let _ = forward.send(InternalEvent::Power(event));
                    }
                })
                .expect("spawn power forward");
        }

        // 手势 tick（15ms）。
        {
            let tick_tx = tx.clone();
            std::thread::Builder::new()
                .name("vh-gesture-tick".into())
                .spawn(move || loop {
                    std::thread::sleep(Duration::from_millis(15));
                    if tick_tx.send(InternalEvent::Tick).is_err() {
                        break;
                    }
                })
                .expect("spawn gesture tick");
        }

        // 主分发。
        {
            let bridge = bridge.clone();
            std::thread::Builder::new()
                .name("vh-dispatcher".into())
                .spawn(move || {
                    while let Ok(event) = rx.recv() {
                        bridge.handle(event);
                    }
                })
                .expect("spawn dispatcher");
        }

        key_gate::install();

        // 恢复上次的端点与连接。
        let (endpoint, device_id, onboarding_done) = {
            let inner = lock(&bridge.inner);
            (
                inner.settings.audio_endpoint_name.clone(),
                inner.settings.paired_device_id.clone(),
                inner.settings.onboarding_complete,
            )
        };
        let direct = bridge.settings().provider.kind == voicehub_core::provider::ProviderKind::SayIt;
        if !direct && endpoint.is_empty() {
            // 未配置过端点：自动挑虚拟声卡候选并写回设置（省一次手动下拉）。
            if let Some(candidate) = bridge
                .audio
                .list_endpoints()
                .unwrap_or_default()
                .iter()
                .find(|e| e.is_virtual_cable_candidate)
            {
                let name = candidate.name.clone();
                let _ = bridge.audio.select_endpoint(candidate.id.clone());
                let mut inner = lock(&bridge.inner);
                inner.settings.audio_endpoint_name = name.clone();
                let settings = inner.settings.clone();
                drop(inner);
                let _ = bridge.store.save_settings(&settings);
                bridge.emit_ui(UiEvent::AudioEndpointChanged { name });
            }
        } else if !direct {
            bridge.restore_endpoint_by_name(&endpoint);
        }
        if let Some(device_id) = device_id {
            if !device_id.is_empty() && onboarding_done {
                bridge.ble.connect(device_id);
            }
        }
        // 启动时按设置同步一次开机自启与实验性续租。
        let extend_enabled = {
            let inner = lock(&bridge.inner);
            inner.settings.experimental_voice_extend
        };
        // The speech workspace reads and changes the shared OS autostart state.
        // Do not overwrite it with the legacy hardware preference on every launch.
        bridge.ble.set_extend_enabled(extend_enabled);
        bridge
    }

    pub fn settings(&self) -> AppSettings {
        lock(&self.inner).settings.clone()
    }

    pub fn poll_remote_voice(&self) -> Option<crate::remote_voice::Packet> {
        lock(&self.remote_voice).poll(now_ms())
    }

    pub fn statistics(&self) -> UsageStatistics {
        lock(&self.inner).statistics.clone()
    }

    fn emit_ui(&self, event: UiEvent) {
        let _ = self.app.emit("bridge://event", &event);
    }

    fn handle(self: &Arc<Self>, event: InternalEvent) {
        match event {
            InternalEvent::Tick => {
                let events = lock(&self.inner).gesture.tick(now_ms());
                for gesture_event in events {
                    self.dispatch_gesture(gesture_event.button, gesture_event.gesture);
                }
            }
            InternalEvent::Hid(input) => self.handle_hid(input),
            InternalEvent::Ble(ble_event) => self.handle_ble(ble_event),
            InternalEvent::Power(power) => match power {
                voicehub_windows::power::PowerEvent::Suspend => {
                    let event = {
                        let _lifecycle = lock(&self.voice_lifecycle);
                        let event = self.reset_voice_locked("System suspended");
                        log::info!("system suspend");
                        event
                    };
                    self.emit_ui(event);
                }
                voicehub_windows::power::PowerEvent::Resume => {
                    log::info!("system resume → reconnect");
                    self.ble.reconnect_now();
                }
            },
        }
    }

    fn handle_hid(self: &Arc<Self>, input: HidInput) {
        let mut inner = lock(&self.inner);
        let events = input.into_events_for(inner.settings.paired_device_id.as_deref());
        let mut gestures = Vec::new();
        let mut counted = false;
        for hid in events {
            match hid {
                HidEvent::UsageSet(usages) => {
                    let now = now_ms();
                    let edges = inner.usage_tracker.update(&usages);
                    for edge in &edges {
                        // 画布实时反馈：沿事件直接推 UI（低频，无需节流）。
                        self.emit_ui(UiEvent::ButtonActivity {
                            button: voicehub_core::mapping::ButtonMapping::key(edge.button),
                            pressed: edge.pressed,
                        });
                    }
                    for edge in edges {
                        let events = if edge.pressed {
                            inner.gesture.press(edge.button, now)
                        } else {
                            inner.gesture.release(edge.button, now)
                        };
                        gestures.extend(events.into_iter().map(|e| (e.button, e.gesture)));
                        if edge.pressed {
                            counted = true;
                            inner.statistics.apply(
                                UsageEvent::ButtonPress {
                                    button_id: voicehub_core::mapping::ButtonMapping::key(edge.button),
                                },
                                chrono::Local::now(),
                            );
                        }
                    }
                }
                HidEvent::VoiceKey { pressed } => {
                    // F5 由 ATVV 控制通道驱动语音（key_gate 负责吞键）；仅记日志。
                    log::debug!("voice key F5 {}", if pressed { "down" } else { "up" });
                }
                HidEvent::WheelClick { button } => {
                    // 滚轮 tick 直接派发，避免连续滚动被双击窗口合并。
                    counted = true;
                    inner.statistics.apply(
                        UsageEvent::ButtonPress {
                            button_id: voicehub_core::mapping::ButtonMapping::key(button),
                        },
                        chrono::Local::now(),
                    );
                    self.emit_ui(UiEvent::ButtonActivity {
                        button: voicehub_core::mapping::ButtonMapping::key(button),
                        pressed: true,
                    });
                    self.emit_ui(UiEvent::ButtonActivity {
                        button: voicehub_core::mapping::ButtonMapping::key(button),
                        pressed: false,
                    });
                    gestures.push((button, Gesture::SingleClick));
                }
                HidEvent::Activity => {}
            }
        }
        drop(inner);
        for (button, gesture) in gestures {
            self.dispatch_gesture(button, gesture);
        }
        if counted {
            self.persist_statistics();
        }
    }

    fn dispatch_gesture(self: &Arc<Self>, button: RemoteButton, gesture: Gesture) {
        let foreground = voicehub_windows::foreground::foreground_process_name();
        let action = {
            let inner = lock(&self.inner);
            if !inner.settings.button_mapping_enabled {
                return;
            }
            inner
                .settings
                .profiles
                .resolve_active(foreground.as_deref())
                .mapping
                .resolve(button, gesture)
        };
        let Some(action) = action else { return };
        let ok = self.execute_action(&action);
        let gesture_label = match gesture {
            Gesture::SingleClick => "单击",
            Gesture::DoubleClick => "双击",
            Gesture::LongPress => "长按",
            Gesture::Repeat => "连发",
        };
        self.emit_ui(UiEvent::ActionReceipt {
            button: voicehub_core::mapping::ButtonMapping::key(button),
            gesture: gesture_label.to_string(),
            action: action_label(&action),
            ok,
        });
        self.persist_statistics();
    }

    fn execute_action(self: &Arc<Self>, action: &ButtonAction) -> bool {
        use voicehub_windows::send_input::{media, tap, volume, volume_mute, KeyChord};
        let result = match action {
            ButtonAction::Disabled => Ok(()),
            ButtonAction::Shortcut { vk, modifiers, .. } => tap(KeyChord::new(*vk, *modifiers)),
            ButtonAction::Custom { shortcut } => tap(KeyChord::new(shortcut.vk, shortcut.modifiers)),
            ButtonAction::MediaKey { code } => media(*code),
            ButtonAction::VolumeUp => volume(false),
            ButtonAction::VolumeDown => volume(true),
            ButtonAction::VolumeMute => volume_mute(),
            ButtonAction::OpenApp { target, .. } | ButtonAction::OpenUrl { url: target } => {
                voicehub_windows::shell::open_target(target)
            }
            ButtonAction::Screenshot { region } => voicehub_windows::shell::screenshot(*region),
            ButtonAction::ShowDesktop => tap(KeyChord::new(0x44, voicehub_core::actions::MOD_WIN)), // Win+D
            ButtonAction::TaskView => tap(KeyChord::new(0x09, voicehub_core::actions::MOD_WIN)), // Win+Tab
            ButtonAction::AppSwitcher => tap(KeyChord::new(0x09, voicehub_core::actions::MOD_ALT)), // Alt+Tab
            ButtonAction::ClickConfirm => voicehub_windows::shell::left_click(),
            ButtonAction::OpenSettings => {
                self.emit_ui(UiEvent::ShowSettings);
                Ok(())
            }
        };
        match result {
            Ok(()) => true,
            Err(error) => {
                log::warn!("action failed: {error}");
                false
            }
        }
    }

    fn handle_ble(self: &Arc<Self>, event: BleEvent) {
        match event {
            BleEvent::SnapshotChanged => {
                let snapshot = self.ble.snapshot();
                let mut events = Vec::new();
                if snapshot.phase != voicehub_windows::ble::ConnectionPhase::Ready {
                    let event = {
                        let _lifecycle = lock(&self.voice_lifecycle);
                        self.reset_voice_locked("Remote disconnected")
                    };
                    events.push(event);
                }
                events.push(UiEvent::BleState { snapshot });
                for event in events { self.emit_ui(event); }
            }
            BleEvent::VoiceStarted { session_id } => { self.on_voice_started(session_id); },
            BleEvent::VoiceStopped { session_id } => self.on_voice_stopped(session_id, None),
            BleEvent::Samples { samples } => self.on_voice_samples(samples, None),
            BleEvent::Battery { percent } => {
                self.emit_ui(UiEvent::Battery { percent });
            }
        }
    }

    fn on_voice_samples(self: &Arc<Self>, samples: Vec<i16>, expected_generation: Option<u64>) {
        // 锁内只做判定与状态变更，UI 事件收集后在锁外 emit——
        // IPC 阻塞不该放大到 lifecycle 锁（15ms tick / HID / BLE 都在等它）。
        let mut events: Vec<UiEvent> = Vec::new();
        {
            let _lifecycle = lock(&self.voice_lifecycle);
            let generation = self.voice_generation.load(Ordering::Relaxed);
            if expected_generation.is_some_and(|expected| expected != generation) { return; }
            if !self.voice_active.load(Ordering::Relaxed) || self.voice_finishing.load(Ordering::Relaxed) { return; }
            let level = voicehub_core::pcm::measure(&samples);
            let direct = lock(&self.voice_provider).as_ref().is_some_and(|(_, p)| p.kind == voicehub_core::provider::ProviderKind::SayIt);
            if direct {
                if !lock(&self.remote_voice).push(generation, &samples) {
                    // 会话已销毁（renderer reload 换 client / 30s 未确认过期 / 主动拒绝）：
                    // 不复位的话 UI 一直显示"录音中"、后续音频全部静默丢失。
                    log::warn!("remote voice session vanished; abandoning generation={generation}");
                    events.push(self.reset_voice_locked("Remote recorder session was reset"));
                }
            } else {
                let _ = self.audio.enqueue_samples(generation, samples);
            }
            if self.level_packet_count.fetch_add(1, Ordering::Relaxed).is_multiple_of(4) {
                events.push(UiEvent::VoiceState { recording: true, level: level.rms });
            }
        }
        for event in events { self.emit_ui(event); }
    }

    /// 复位语音会话（必须在持有 voice_lifecycle 锁时调用）。
    /// 返回给 UI 的收尾事件，由调用方在锁外 emit。
    fn reset_voice_locked(self: &Arc<Self>, cancel_reason: &str) -> UiEvent {
        lock(&self.remote_voice).cancel(cancel_reason);
        self.voice_generation.fetch_add(1, Ordering::Relaxed);
        self.voice_active.store(false, Ordering::Relaxed);
        self.voice_finishing.store(false, Ordering::Relaxed);
        if let Some((_, provider)) = lock(&self.voice_provider).take() {
            self.trigger_provider(provider.trigger_on_stream_stop());
        }
        UiEvent::VoiceState { recording: false, level: 0.0 }
    }

    fn on_voice_started(self: &Arc<Self>, session_id: u8) -> Option<u64> {
        let voice_event;
        let generation;
        {
            let _lifecycle = lock(&self.voice_lifecycle);
            if self.voice_active.load(Ordering::Relaxed) && !self.voice_finishing.load(Ordering::Relaxed) {
                return None;
            }
            // drain 窗口内新会话：同步完成旧收尾再开新会话。sleep 中的 finisher
            // 醒来会发现 generation 已变而自动放弃。不这样做的话，BLE 丢 start 包
            // 走隐式补开流或用户极速重按（<drain_ms）时，第二段语音会整段丢失。
            if self.voice_finishing.load(Ordering::Relaxed) {
                let old_generation = self.voice_generation.load(Ordering::Relaxed);
                let old_started_at = self.voice_started_at_ms.load(Ordering::Relaxed);
                let old_duration_ms = now_ms().saturating_sub(old_started_at);
                let stop_trigger = lock(&self.voice_provider).take()
                    .map(|(_, p)| p.trigger_on_stream_stop());
                let _ = self.audio.finish_session(old_generation);
                self.voice_finishing.store(false, Ordering::Relaxed);
                if let Some(trigger) = stop_trigger {
                    self.trigger_provider(trigger);
                }
                self.record_voice_session_since(old_started_at, old_duration_ms, 0);
                log::info!("voice restarted during drain; finished generation={old_generation} inline");
            }
            generation = self.voice_generation.fetch_add(1, Ordering::Relaxed) + 1;
            self.voice_started_at_ms.store(now_ms(), Ordering::Relaxed);
            self.voice_active.store(true, Ordering::Relaxed);
            let provider = lock(&self.inner).settings.provider.clone();
            *lock(&self.voice_provider) = Some((session_id, provider.clone()));
            if provider.kind == voicehub_core::provider::ProviderKind::SayIt {
                if !lock(&self.remote_voice).begin(generation, now_ms()) {
                    log::warn!("Remote recorder busy; ignored generation={generation}");
                    self.voice_active.store(false, Ordering::Relaxed);
                    *lock(&self.voice_provider) = None;
                    drop(_lifecycle);
                    let _ = self.app.emit("voicehub-voice-error", "上一段录音正在处理，请稍后重试");
                    return None;
                }
            } else {
                let _ = self.audio.begin_session(generation);
                self.trigger_provider(provider.trigger_on_stream_start());
            }
            log::info!("voice start session={session_id} generation={generation}");
            voice_event = UiEvent::VoiceState { recording: true, level: 0.0 };
        }
        self.emit_ui(voice_event);
        Some(generation)
    }

    fn on_voice_stopped(self: &Arc<Self>, session_id: u8, expected_generation: Option<u64>) {
        enum Stop {
            /// 直连路径：样本已全在 RemoteVoice 缓冲，end 后 renderer drain，立即收尾。
            DirectDone(UiEvent),
            /// 外部 Provider：保留会话至 drain 窗口结束（把尾巴样本灌满虚拟声卡）。
            Drain { generation: u64, started_at: u64, duration_ms: u64, provider: voicehub_core::provider::ProviderConfig },
        }
        let stop = {
            let _lifecycle = lock(&self.voice_lifecycle);
            if expected_generation.is_some_and(|expected| expected != self.voice_generation.load(Ordering::Relaxed)) { return; }
            if self.voice_finishing.load(Ordering::Relaxed) { return; }
            let provider = {
                let current = lock(&self.voice_provider);
                if current.as_ref().is_none_or(|(id, _)| *id != session_id) { return; }
                current.as_ref().unwrap().1.clone()
            };
            let generation = self.voice_generation.load(Ordering::Relaxed);
            let started_at = self.voice_started_at_ms.load(Ordering::Relaxed);
            let duration_ms = now_ms().saturating_sub(started_at);
            if provider.kind == voicehub_core::provider::ProviderKind::SayIt {
                let sample_count = lock(&self.remote_voice).received() as u64;
                lock(&self.remote_voice).end(generation);
                *lock(&self.voice_provider) = None;
                self.voice_active.store(false, Ordering::Relaxed);
                log::info!("voice stop session={session_id} duration_ms={duration_ms} samples={sample_count}");
                self.record_voice_session_since(started_at, duration_ms, sample_count);
                Stop::DirectDone(UiEvent::VoiceState { recording: false, level: 0.0 })
            } else {
                self.voice_finishing.store(true, Ordering::Relaxed);
                Stop::Drain { generation, started_at, duration_ms, provider }
            }
        };
        match stop {
            Stop::DirectDone(event) => self.emit_ui(event),
            Stop::Drain { generation, started_at, duration_ms, provider } => {
                let drain_ms = provider.drain_ms() as u64;
                let bridge = self.clone();
                let provider_for_thread = provider.clone();
                let spawned = std::thread::Builder::new()
                    .name("vh-voice-finish".into())
                    .spawn(move || {
                        std::thread::sleep(Duration::from_millis(drain_ms));
                        bridge.finish_drained_session(generation, session_id, &provider_for_thread, started_at, duration_ms);
                    });
                if spawned.is_err() {
                    // 线程资源耗尽的极端场景：放弃等待、同步立即收尾——
                    // 卡在 finishing 的话后续语音会被 busy 检查全部拒绝。
                    log::error!("spawn voice finisher failed; finishing generation={generation} inline without drain");
                    self.finish_drained_session(generation, session_id, &provider, started_at, duration_ms);
                }
            }
        }
    }

    /// drain 结束（或 spawn 失败降级）后的统一收尾：完成音频会话、触发停止键、记录历史。
    fn finish_drained_session(
        self: &Arc<Self>,
        generation: u64,
        session_id: u8,
        provider: &voicehub_core::provider::ProviderConfig,
        started_at: u64,
        duration_ms: u64,
    ) {
        let event = {
            let _lifecycle = lock(&self.voice_lifecycle);
            // drain 期间被新会话 inline 收尾（on_voice_started）或断连复位：generation
            // 已变，本会话状态早被接管/清理，这里不再动任何共享状态。
            if self.voice_generation.load(Ordering::Relaxed) != generation { return; }
            let _ = self.audio.finish_session(generation);
            let trigger = provider.trigger_on_stream_stop();
            self.trigger_provider(trigger);
            *lock(&self.voice_provider) = None;
            self.voice_active.store(false, Ordering::Relaxed);
            self.voice_finishing.store(false, Ordering::Relaxed);
            log::info!("voice stop session={session_id} duration_ms={duration_ms}");
            self.record_voice_session_since(started_at, duration_ms, 0);
            UiEvent::VoiceState { recording: false, level: 0.0 }
        };
        self.emit_ui(event);
    }

    /// 记录一次语音会话到统计与历史。
    /// sample_count：直连路径为 RemoteVoice 实收值；外部 Provider 路径音频直接进
    /// 声卡、不经我们计数，记 0。
    fn record_voice_session_since(self: &Arc<Self>, started_at: u64, duration_ms: u64, sample_count: u64) {
        let foreground = voicehub_windows::foreground::foreground_process_name();
        let profile_name = {
            let inner = lock(&self.inner);
            inner
                .settings
                .profiles
                .resolve_active(foreground.as_deref())
                .name
                .clone()
        };
        {
            let mut inner = lock(&self.inner);
            inner.statistics.apply(
                UsageEvent::VoiceSession { duration_ms },
                chrono::Local::now(),
            );
        }
        let record = VoiceSessionRecord {
            started_at_ms: started_at as i64,
            duration_ms,
            sample_count,
            foreground_process: foreground,
            profile_name,
        };
        let _ = self.store.append_history(&record);
        self.persist_statistics();
    }

    fn trigger_provider(self: &Arc<Self>, trigger: ProviderTrigger) {
        use voicehub_windows::send_input::{press, release, tap, KeyChord};
        let result = match trigger {
            ProviderTrigger::Tap { vk, modifiers } => tap(KeyChord::new(vk, modifiers)),
            ProviderTrigger::Press { vk, modifiers } => press(KeyChord::new(vk, modifiers)),
            ProviderTrigger::Release { vk, modifiers } => release(KeyChord::new(vk, modifiers)),
            ProviderTrigger::None => Ok(()),
        };
        if let Err(error) = result {
            log::warn!("provider trigger failed: {error}");
        }
    }

    pub fn persist_statistics(&self) {
        let stats = lock(&self.inner).statistics.clone();
        if let Err(error) = self.store.save_statistics(&stats) {
            log::warn!("statistics save failed: {error}");
        }
    }

    pub fn apply_settings(self: &Arc<Self>, settings: AppSettings) -> Result<(), String> {
        self.apply_settings_with(settings, true)
    }

    /// `restore_audio=false`：调用方（select_audio_endpoint）已真实打开新端点，
    /// 不再走 restore——释放再重开会在同线 pin 互斥窗口里被外部进程抢线。
    pub fn apply_settings_with(self: &Arc<Self>, settings: AppSettings, restore_audio: bool) -> Result<(), String> {
        let previous;
        let audio_changed;
        let autostart_changed;
        let language_changed;
        let extend_changed;
        {
            let mut inner = lock(&self.inner);
            // 锁内只 diff + 更新内存；写盘（含 sync_all）挪到锁外——锁内写盘会把
            // 15ms tick / HID / BLE 事件分发一起阻塞在磁盘延迟上。
            previous = inner.settings.clone();
            audio_changed = restore_audio && settings.provider.kind != voicehub_core::provider::ProviderKind::SayIt &&
                (settings.audio_endpoint_name != previous.audio_endpoint_name || settings.provider.kind != previous.provider.kind);
            autostart_changed = settings.launch_at_login != previous.launch_at_login;
            language_changed = settings.language != previous.language;
            extend_changed = settings.experimental_voice_extend != previous.experimental_voice_extend;
            if settings.paired_device_id != previous.paired_device_id {
                inner.gesture.reset();
                inner.usage_tracker = UsageTracker::default();
            }
            inner.settings = settings.clone();
        }
        if let Err(error) = self.store.save_settings(&settings) {
            // 写盘失败：内存回滚并上抛。等值检查防止把期间另一路 apply 的新值误回滚。
            let mut inner = lock(&self.inner);
            if inner.settings == settings {
                inner.settings = previous;
            }
            return Err(error.to_string());
        }
        self.set_gain(settings.gain_db);
        if audio_changed {
            self.restore_endpoint_by_name(&settings.audio_endpoint_name);
        }
        if autostart_changed {
            self.sync_autostart(settings.launch_at_login);
        }
        if language_changed {
            crate::refresh_tray_menu(&self.app);
        }
        if extend_changed {
            self.ble.set_extend_enabled(settings.experimental_voice_extend);
        }
        Ok(())
    }

    /// 开机自启与插件状态同步（失败只记日志，不阻塞设置保存）。
    /// disable 前先查状态：快捷方式本就不存在时 disable 会报 os error 2，属幂等场景而非故障。
    fn sync_autostart(self: &Arc<Self>, enable: bool) {
        use tauri_plugin_autostart::ManagerExt;
        let autolaunch = self.app.autolaunch();
        if !enable && !autolaunch.is_enabled().unwrap_or(false) {
            return;
        }
        let result = if enable {
            autolaunch.enable()
        } else {
            autolaunch.disable()
        };
        if let Err(error) = result {
            log::warn!("autostart sync failed (enable={enable}): {error}");
        }
    }

    fn restore_endpoint_by_name(self: &Arc<Self>, name: &str) {
        if name.is_empty() {
            return;
        }
        let endpoints = self.audio.list_endpoints().unwrap_or_default();
        let is_stereo_cable = |e: &voicehub_windows::AudioEndpoint| {
            e.is_virtual_cable_candidate && e.name.to_lowercase().contains("cable input")
        };
        let exact = endpoints.iter().find(|e| e.name == name);
        let stereo = endpoints.iter().find(|e| is_stereo_cable(e));
        // 多通道变体（"CABLE In 16ch" 等）没有 capture 端，语音工具收不到声，
        // 且与标准 2ch pin 互斥、长期持有会把 2ch 锁成 DEVICE_IN_USE。
        // 存在标准 2ch 端点时自动迁移过去并写回设置（2026-09-06 实测结论）。
        let chosen = match (exact, stereo) {
            (Some(current), Some(stereo))
                if current.is_virtual_cable_candidate && !is_stereo_cable(current) =>
            {
                log::info!(
                    "端点「{name}」是多通道变体且无 capture 端，迁移到「{}」",
                    stereo.name
                );
                Some(stereo)
            }
            (Some(current), _) => Some(current),
            (None, _) => endpoints.iter().find(|e| e.is_virtual_cable_candidate),
        };
        if let Some(endpoint) = chosen {
            let _ = self.audio.select_endpoint(endpoint.id.clone());
            // 实际端点与设置里的名字不一致（迁移或 fallback）时写回，
            // 否则每次启动都重走一遍查找。
            if endpoint.name != name {
                let mut inner = lock(&self.inner);
                inner.settings.audio_endpoint_name = endpoint.name.clone();
                let settings = inner.settings.clone();
                drop(inner);
                if let Err(error) = self.store.save_settings(&settings) {
                    log::warn!("endpoint 迁移写回失败：{error}");
                }
            }
            self.emit_ui(UiEvent::AudioEndpointChanged { name: endpoint.name.clone() });
        } else {
            log::warn!("audio endpoint not found: {name}");
        }
    }

    /// 模拟遥控器页：直接喂入手势（与真实 HID 同一条分发路径）。
    pub fn simulate_gesture(self: &Arc<Self>, button: RemoteButton, gesture: Gesture) {
        self.dispatch_gesture(button, gesture);
    }

    /// 模拟遥控器页：合成一段语音（440→880Hz 扫频，走真实音频管线到端点）。
    pub fn simulate_voice(self: &Arc<Self>, duration_ms: u64, pcm: Option<Vec<i16>>) -> Result<(), String> {
        let generation = self.on_voice_started(0xFE).ok_or("Recorder is busy or previous recording is still processing")?;
        let bridge = self.clone();
        std::thread::Builder::new()
            .name("vh-sim-voice".into())
            .spawn(move || {
                let sample_rate = 16_000u64;
                let total = pcm.as_ref().map_or((sample_rate * duration_ms / 1000).max(1), |samples| samples.len() as u64);
                let chunk = sample_rate / 10; // 100ms 一包
                // 长音频（>4 分钟）按 10 倍速快放：实时回放 5 分钟会逼近 remote_voice
                // 的 360s 会话超时，只剩 45s 余量；快放让模拟器能测完整长音频。
                let pace_ms: u64 = if total > sample_rate * 240 { 10 } else { 100 };
                let mut written = 0u64;
                while written < total {
                    if bridge.voice_generation.load(Ordering::Relaxed) != generation || !bridge.voice_active.load(Ordering::Relaxed) { return; }
                    let count = chunk.min(total - written);
                    let mut samples = Vec::with_capacity(count as usize);
                    for i in 0..count {
                        let index = written + i;
                        if let Some(pcm) = &pcm { samples.push(pcm[index as usize]); continue; }
                        let t = index as f64 / sample_rate as f64;
                        let freq = 440.0 + 440.0 * index as f64 / total as f64;
                        let envelope = 0.6 + 0.4 * (2.0 * std::f64::consts::PI * 2.0 * t).sin();
                        let value = (2.0 * std::f64::consts::PI * freq * t).sin() * 0.4 * envelope;
                        samples.push((value * 32767.0) as i16);
                    }
                    bridge.on_voice_samples(samples, Some(generation));
                    written += count;
                    std::thread::sleep(Duration::from_millis(pace_ms));
                }
                bridge.on_voice_stopped(0xFE, Some(generation));
            })
            .expect("spawn sim voice");
        Ok(())
    }

    pub fn set_gain(&self, gain_db: f64) {
        self.ble.set_gain(gain_db);
    }
}

fn action_label(action: &ButtonAction) -> String {
    match action {
        ButtonAction::Disabled => "未绑定".into(),
        ButtonAction::Shortcut { label, .. } => label.clone(),
        ButtonAction::Custom { shortcut } => shortcut.label.clone(),
        ButtonAction::MediaKey { code } => format!("{code:?}"),
        ButtonAction::VolumeUp => "音量+".into(),
        ButtonAction::VolumeDown => "音量−".into(),
        ButtonAction::VolumeMute => "静音".into(),
        ButtonAction::OpenApp { label, .. } => format!("打开 {label}"),
        ButtonAction::OpenUrl { url } => format!("打开 {url}"),
        ButtonAction::Screenshot { region: true } => "区域截图".into(),
        ButtonAction::Screenshot { region: false } => "全屏截图".into(),
        ButtonAction::ShowDesktop => "显示桌面".into(),
        ButtonAction::TaskView => "任务视图".into(),
        ButtonAction::AppSwitcher => "切换应用".into(),
        ButtonAction::ClickConfirm => "点击确认".into(),
        ButtonAction::OpenSettings => "打开声桥".into(),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 前后端事件契约：tag 值必须与前端 types.ts 的判别字符串完全一致。
    /// 前端对应清单见 src/i18n 无关、src/types.ts UiEvent 与各页 switch。
    #[test]
    fn ui_event_tags_match_frontend_contract() {
        let cases: Vec<(UiEvent, &str)> = vec![
            (
                UiEvent::BleState {
                    snapshot: voicehub_windows::ble::BleSnapshot::default(),
                },
                "BleState",
            ),
            (
                UiEvent::VoiceState {
                    recording: true,
                    level: 0.5,
                },
                "VoiceState",
            ),
            (UiEvent::Battery { percent: 80 }, "Battery"),
            (
                UiEvent::ActionReceipt {
                    button: "ok".into(),
                    gesture: "single".into(),
                    action: "音量+".into(),
                    ok: true,
                },
                "ActionReceipt",
            ),
            (
                UiEvent::ButtonActivity {
                    button: "ok".into(),
                    pressed: true,
                },
                "ButtonActivity",
            ),
            (UiEvent::ShowSettings, "ShowSettings"),
            (
                UiEvent::AudioEndpointChanged {
                    name: "CABLE Input".into(),
                },
                "AudioEndpointChanged",
            ),
        ];
        for (event, expected_tag) in cases {
            let value = serde_json::to_value(&event).unwrap();
            assert_eq!(value["type"], expected_tag, "UiEvent tag 契约被破坏");
        }

        // 字段名也是契约的一部分（前端直接读 payload.snapshot / .recording 等）。
        let ble = serde_json::to_value(UiEvent::BleState {
            snapshot: voicehub_windows::ble::BleSnapshot::default(),
        })
        .unwrap();
        assert!(ble.get("snapshot").is_some(), "BleState 缺 snapshot 字段");
        let voice = serde_json::to_value(UiEvent::VoiceState {
            recording: false,
            level: 0.0,
        })
        .unwrap();
        assert!(voice.get("recording").is_some() && voice.get("level").is_some());
        let receipt = serde_json::to_value(UiEvent::ActionReceipt {
            button: "ok".into(),
            gesture: "single".into(),
            action: "音量+".into(),
            ok: true,
        })
        .unwrap();
        for field in ["button", "gesture", "action", "ok"] {
            assert!(receipt.get(field).is_some(), "ActionReceipt 缺 {field} 字段");
        }
    }
}
