/// 与 Rust 侧 serde 类型一一镜像（camelCase）。

export type Language = "system" | "zh_cn" | "english";
export type Theme = "system" | "light" | "dark";
export type ProviderKind = "we_type" | "doubao" | "win_h" | "custom" | "none";
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
  | { kind: "open_settings" }
  | { kind: "custom"; shortcut: CustomShortcut };

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
  | { type: "ShowSettings" }
  | { type: "AudioEndpointChanged"; name: string };
