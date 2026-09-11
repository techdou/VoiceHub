/// 与 Rust 侧 serde 类型一一镜像（camelCase）。

export type Language = "system" | "zh_cn" | "english";
export type Theme = "system" | "light" | "dark";
export type ProviderKind = "sayit" | "custom" | "none";
export type TriggerMode = "toggle" | "hold";

export interface CustomShortcut {
  vk: number;
  modifiers: number;
  label: string;
}

export type ButtonAction =
  | { kind: "disabled" }
  | { kind: "shortcut"; vk: number; modifiers: number; label: string }
  | { kind: "media_key"; code: "play_pause" | "stop" | "next" | "previous" | "mute" }
  | { kind: "volume_up" }
  | { kind: "volume_down" }
  | { kind: "volume_mute" }
  | { kind: "open_app"; target: string; label: string }
  | { kind: "open_url"; url: string }
  | { kind: "screenshot"; region: boolean }
  | { kind: "show_desktop" }
  | { kind: "task_view" }
  | { kind: "app_switcher" }
  | { kind: "click_confirm" }
  | { kind: "delete_line" }
  | { kind: "open_settings" }
  | { kind: "custom"; shortcut: CustomShortcut }
  /** 免提触发（事件直连引擎，不注入按键）。 */
  | { kind: "trigger_hands_free" };

export type RemoteButtonId =
  | "power"
  | "up"
  | "left"
  | "ok"
  | "right"
  | "down"
  | "back"
  | "volume_up"
  | "home"
  | "volume_down"
  | "menu"
  | "tv";

export interface ButtonBinding {
  single: ButtonAction;
  double: ButtonAction;
  long: ButtonAction;
  /** 边沿直达的按住说话触发键（事件直连引擎，Rust 端 serde default false）。 */
  pushToTalk: boolean;
}

export interface ButtonMapping {
  bindings: Record<string, ButtonBinding>;
}

export interface ButtonProfile {
  id: string;
  name: string;
  icon: string;
  mapping: ButtonMapping;
}

export interface ProfileRules {
  processBindings: Record<string, string>;
  fallbackProfileId: string;
}

export interface ProfileStore {
  profiles: ButtonProfile[];
  selectedProfileId: string;
  smartEnabled: boolean;
  rules: ProfileRules;
}

export interface ProviderConfig {
  kind: ProviderKind;
  customVk: number;
  customModifiers: number;
  customMode: TriggerMode;
  /** SayIt 触发键主键（0 = 默认右 Alt）；与 customVk 互不干扰 */
  sayitVk: number;
  /** SayIt 触发键修饰键（0 = 单键）。SayIt 过滤注入的单键，程序联动必须用组合键。
   *  Rust 端（provider.rs）必填且总是序列化，这里不带 ?（mirror 注释见文件头）。 */
  sayitModifiers: number;
  stopDelayMs: number;
  startupGraceMs: number;
}

export interface AppSettings {
  schemaVersion: number;
  onboardingComplete: boolean;
  pairedDeviceId: string | null;
  pairedDeviceName: string | null;
  audioEndpointName: string;
  gainDb: number;
  provider: ProviderConfig;
  profiles: ProfileStore;
  buttonMappingEnabled: boolean;
  /** Rust 端（settings.rs）必填且总是序列化。 */
  experimentalVoiceExtend: boolean;
  /** 录音键触发模式：ptt=按住说话（蓝牙直传）；hands_free=按一下开关录音（系统麦克风）。 */
  voiceKeyTriggerMode: "ptt" | "hands_free";
  /** F5 拦截总开关：遥控器在线期间吞掉全部 F5（含真键盘），防止语音键泄漏刷新前台页面。 */
  f5GateEnabled: boolean;
  launchAtLogin: boolean;
  language: Language;
  theme: Theme;
}

export type ConnectionPhase =
  | "stopped"
  | "scanning"
  | "connecting"
  | "discovering"
  | "ready"
  | "reconnecting"
  | "failed";

export interface BleSnapshot {
  phase: ConnectionPhase;
  remoteName: string | null;
  remoteModel: string | null;
  batteryPercent: number | null;
  voiceStreaming: boolean;
  sessionId: number;
  consecutiveFailures: number;
  radioCycles: number;
  lastError: string | null;
}

export interface PairedRemote {
  id: string;
  name: string;
}

export interface AudioEndpoint {
  id: string;
  name: string;
  isVirtualCableCandidate: boolean;
}

export interface DailyUsage {
  buttonPressCount: number;
  buttonCounts: Record<string, number>;
  voiceSessionCount: number;
  voiceDurationMs: number;
  longestVoiceMs: number;
}

export interface UsageStatistics {
  days: Record<string, DailyUsage>;
}

export interface VoiceSessionRecord {
  startedAtMs: number;
  durationMs: number;
  sampleCount: number;
  foregroundProcess: string | null;
  profileName: string;
}

export interface DiagnosticItem {
  id: string;
  title: string;
  detail: string;
  status: "ok" | "warn" | "fail" | "info";
}

export interface CableInstallState {
  state: string;
  detail: string;
  exitCode: number;
}

export interface CableStatus {
  endpointPresent: boolean;
  servicePresent: boolean;
  installState: CableInstallState | null;
  busy: boolean;
}

export type UiEvent =
  | { type: "BleState"; snapshot: BleSnapshot }
  | { type: "VoiceState"; recording: boolean; level: number }
  | { type: "Battery"; percent: number }
  | {
      type: "ActionReceipt";
      button: string;
      gesture: string;
      action: string;
      ok: boolean;
    }
  | { type: "ButtonActivity"; button: string; pressed: boolean }
  | { type: "ShowSettings" }
  | { type: "AudioEndpointChanged"; name: string };
