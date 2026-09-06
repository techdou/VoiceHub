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

use sb_core::actions::ButtonAction;
use sb_core::buttons::RemoteButton;
use sb_core::gesture::{Gesture, GestureRecognizer};
use sb_core::provider::ProviderTrigger;
use sb_core::settings::{AppSettings, VoiceSessionRecord};
use sb_core::statistics::{UsageEvent, UsageStatistics};

use crate::store::Store;
use sb_windows::audio::AudioRuntime;
use sb_windows::ble::{BleEvent, BleRuntime};
use sb_windows::key_gate;
use sb_windows::raw_input::{spawn_hid_monitor, HidEvent, UsageTracker};

enum InternalEvent {
    Hid(HidEvent),
    Ble(BleEvent),
    Power(sb_windows::power::PowerEvent),
    /// 手势 tick（驱动单击窗口/长按判定）。
    Tick,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum UiEvent {
    BleState { snapshot: sb_windows::ble::BleSnapshot },
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
    level_packet_count: AtomicU64,
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
                .name("sb-ble-forward".into())
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
            level_packet_count: AtomicU64::new(0),
        });

        // HID 监视。
        {
            let (hid_tx, hid_rx) = channel::<HidEvent>();
            let _ = spawn_hid_monitor(hid_tx);
            let forward = tx.clone();
            std::thread::Builder::new()
                .name("sb-hid-forward".into())
                .spawn(move || {
                    while let Ok(event) = hid_rx.recv() {
                        let _ = forward.send(InternalEvent::Hid(event));
                    }
                })
                .expect("spawn hid forward");
        }

        // 电源通知。
        {
            let (power_tx, power_rx) = channel::<sb_windows::power::PowerEvent>();
            let _ = sb_windows::power::spawn_power_monitor(power_tx);
            let forward = tx.clone();
            std::thread::Builder::new()
                .name("sb-power-forward".into())
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
                .name("sb-gesture-tick".into())
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
                .name("sb-dispatcher".into())
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
        if endpoint.is_empty() {
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
        } else {
            bridge.restore_endpoint_by_name(&endpoint);
        }
        if let Some(device_id) = device_id {
            if !device_id.is_empty() && onboarding_done {
                bridge.ble.connect(device_id);
            }
        }
        // 启动时按设置同步一次开机自启与实验性续租。
        let (autostart_enabled, extend_enabled) = {
            let inner = lock(&bridge.inner);
            (inner.settings.launch_at_login, inner.settings.experimental_voice_extend)
        };
        bridge.sync_autostart(autostart_enabled);
        bridge.ble.set_extend_enabled(extend_enabled);
        bridge
    }

    pub fn settings(&self) -> AppSettings {
        lock(&self.inner).settings.clone()
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
            InternalEvent::Hid(hid) => self.handle_hid(hid),
            InternalEvent::Ble(ble_event) => self.handle_ble(ble_event),
            InternalEvent::Power(power) => match power {
                sb_windows::power::PowerEvent::Suspend => {
                    log::info!("system suspend");
                }
                sb_windows::power::PowerEvent::Resume => {
                    log::info!("system resume → reconnect");
                    self.ble.reconnect_now();
                }
            },
        }
    }

    fn handle_hid(self: &Arc<Self>, hid: HidEvent) {
        match hid {
            HidEvent::UsageSet(usages) => {
                let now = now_ms();
                let mut gestures = Vec::new();
                {
                    let mut inner = lock(&self.inner);
                    let edges = inner.usage_tracker.update(&usages);
                    for edge in &edges {
                        // 画布实时反馈：沿事件直接推 UI（低频，无需节流）。
                        self.emit_ui(UiEvent::ButtonActivity {
                            button: sb_core::mapping::ButtonMapping::key(edge.button),
                            pressed: edge.pressed,
                        });
                    }
                    for edge in edges {
                        let events = if edge.pressed {
                            inner.gesture.press(edge.button, now)
                        } else {
                            inner.gesture.release(edge.button, now)
                        };
                        gestures.extend(events);
                        if edge.pressed {
                            inner.statistics.apply(
                                UsageEvent::ButtonPress {
                                    button_id: sb_core::mapping::ButtonMapping::key(edge.button),
                                },
                                chrono::Local::now(),
                            );
                        }
                    }
                }
                for gesture_event in gestures {
                    self.dispatch_gesture(gesture_event.button, gesture_event.gesture);
                }
            }
            HidEvent::VoiceKey { pressed } => {
                // F5 由 ATVV 控制通道驱动语音（key_gate 负责吞键）；仅记日志。
                log::debug!("voice key F5 {}", if pressed { "down" } else { "up" });
            }
            HidEvent::Activity => {}
        }
    }

    fn dispatch_gesture(self: &Arc<Self>, button: RemoteButton, gesture: Gesture) {
        let foreground = sb_windows::foreground::foreground_process_name();
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
            button: sb_core::mapping::ButtonMapping::key(button),
            gesture: gesture_label.to_string(),
            action: action_label(&action),
            ok,
        });
        self.persist_statistics();
    }

    fn execute_action(self: &Arc<Self>, action: &ButtonAction) -> bool {
        use sb_windows::send_input::{media, tap, volume, volume_mute, KeyChord};
        let result = match action {
            ButtonAction::Disabled => Ok(()),
            ButtonAction::Shortcut { vk, modifiers, .. } => tap(KeyChord::new(*vk, *modifiers)),
            ButtonAction::Custom { shortcut } => tap(KeyChord::new(shortcut.vk, shortcut.modifiers)),
            ButtonAction::MediaKey { code } => media(*code),
            ButtonAction::VolumeUp => volume(false),
            ButtonAction::VolumeDown => volume(true),
            ButtonAction::VolumeMute => volume_mute(),
            ButtonAction::OpenApp { target, .. } | ButtonAction::OpenUrl { url: target } => {
                sb_windows::shell::open_target(target)
            }
            ButtonAction::Screenshot { region } => sb_windows::shell::screenshot(*region),
            ButtonAction::ShowDesktop => tap(KeyChord::new(0x44, sb_core::actions::MOD_WIN)), // Win+D
            ButtonAction::TaskView => tap(KeyChord::new(0x09, sb_core::actions::MOD_WIN)), // Win+Tab
            ButtonAction::AppSwitcher => tap(KeyChord::new(0x09, sb_core::actions::MOD_ALT)), // Alt+Tab
            ButtonAction::ClickConfirm => sb_windows::shell::left_click(),
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
                self.emit_ui(UiEvent::BleState { snapshot });
            }
            BleEvent::VoiceStarted { session_id } => self.on_voice_started(session_id),
            BleEvent::VoiceStopped { session_id } => self.on_voice_stopped(session_id),
            BleEvent::Samples { samples } => {
                let level = sb_core::pcm::measure(&samples);
                let generation = self.voice_generation.load(Ordering::Relaxed);
                let _ = self.audio.enqueue_samples(generation, samples);
                // 每 4 包（~40ms）推一次电平，避免 UI 洪泛。
                if self.level_packet_count.fetch_add(1, Ordering::Relaxed) % 4 == 0 {
                    self.emit_ui(UiEvent::VoiceState {
                        recording: self.voice_active.load(Ordering::Relaxed),
                        level: level.rms,
                    });
                }
            }
            BleEvent::Battery { percent } => {
                self.emit_ui(UiEvent::Battery { percent });
            }
        }
    }

    fn on_voice_started(self: &Arc<Self>, session_id: u8) {
        let generation = self.voice_generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.voice_started_at_ms.store(now_ms(), Ordering::Relaxed);
        self.voice_active.store(true, Ordering::Relaxed);
        let _ = self.audio.begin_session(generation);
        let trigger = lock(&self.inner).settings.provider.trigger_on_stream_start();
        self.trigger_provider(trigger);
        log::info!("voice start session={session_id} generation={generation}");
        self.emit_ui(UiEvent::VoiceState { recording: true, level: 0.0 });
    }

    fn on_voice_stopped(self: &Arc<Self>, session_id: u8) {
        self.voice_active.store(false, Ordering::Relaxed);
        let generation = self.voice_generation.load(Ordering::Relaxed);
        let started_at = self.voice_started_at_ms.load(Ordering::Relaxed);
        let duration_ms = now_ms().saturating_sub(started_at);
        let drain_ms = lock(&self.inner).settings.provider.drain_ms();

        let bridge = self.clone();
        std::thread::Builder::new()
            .name("sb-voice-finish".into())
            .spawn(move || {
                std::thread::sleep(Duration::from_millis(drain_ms as u64));
                let _ = bridge.audio.finish_session(generation);
                let trigger = lock(&bridge.inner).settings.provider.trigger_on_stream_stop();
                bridge.trigger_provider(trigger);
                log::info!("voice stop session={session_id} duration_ms={duration_ms}");
                bridge.record_voice_session(duration_ms);
                bridge.emit_ui(UiEvent::VoiceState { recording: false, level: 0.0 });
            })
            .expect("spawn voice finisher");
    }

    fn record_voice_session(self: &Arc<Self>, duration_ms: u64) {
        let foreground = sb_windows::foreground::foreground_process_name();
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
            started_at_ms: self.voice_started_at_ms.load(Ordering::Relaxed) as i64,
            duration_ms,
            sample_count: 0,
            foreground_process: foreground,
            profile_name,
        };
        let _ = self.store.append_history(&record);
        self.persist_statistics();
    }

    fn trigger_provider(self: &Arc<Self>, trigger: ProviderTrigger) {
        use sb_windows::send_input::{press, release, tap, KeyChord};
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

    pub fn apply_settings(self: &Arc<Self>, settings: AppSettings) {
        let audio_changed;
        let autostart_changed;
        let language_changed;
        let extend_changed;
        {
            let mut inner = lock(&self.inner);
            audio_changed = settings.audio_endpoint_name != inner.settings.audio_endpoint_name;
            autostart_changed = settings.launch_at_login != inner.settings.launch_at_login;
            language_changed = settings.language != inner.settings.language;
            extend_changed = settings.experimental_voice_extend != inner.settings.experimental_voice_extend;
            inner.settings = settings.clone();
        }
        let _ = self.store.save_settings(&settings);
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
    }

    /// 开机自启与插件状态同步（失败只记日志，不阻塞设置保存）。
    fn sync_autostart(self: &Arc<Self>, enable: bool) {
        use tauri_plugin_autostart::ManagerExt;
        let autolaunch = self.app.autolaunch();
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
        let found = endpoints
            .iter()
            .find(|e| e.name == name)
            .or_else(|| endpoints.iter().find(|e| e.is_virtual_cable_candidate));
        if let Some(endpoint) = found {
            let _ = self.audio.select_endpoint(endpoint.id.clone());
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
    pub fn simulate_voice(self: &Arc<Self>, duration_ms: u64) {
        self.on_voice_started(0xFE);
        let bridge = self.clone();
        std::thread::Builder::new()
            .name("sb-sim-voice".into())
            .spawn(move || {
                let sample_rate = 16_000u64;
                let total = (sample_rate * duration_ms / 1000).max(1);
                let chunk = sample_rate / 10; // 100ms 一包
                let mut written = 0u64;
                while written < total {
                    let count = chunk.min(total - written);
                    let mut samples = Vec::with_capacity(count as usize);
                    for i in 0..count {
                        let index = written + i;
                        let t = index as f64 / sample_rate as f64;
                        let freq = 440.0 + 440.0 * index as f64 / total as f64;
                        let envelope = 0.6 + 0.4 * (2.0 * std::f64::consts::PI * 2.0 * t).sin();
                        let value = (2.0 * std::f64::consts::PI * freq * t).sin() * 0.4 * envelope;
                        samples.push((value * 32767.0) as i16);
                    }
                    let generation = bridge.voice_generation.load(Ordering::Relaxed);
                    let _ = bridge.audio.enqueue_samples(generation, samples);
                    written += count;
                    std::thread::sleep(Duration::from_millis(100));
                }
                bridge.on_voice_stopped(0xFE);
            })
            .expect("spawn sim voice");
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
