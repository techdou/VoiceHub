import { invoke } from "@tauri-apps/api/core";
import type {
  AppSettings,
  AudioEndpoint,
  BleSnapshot,
  DiagnosticItem,
  PairedRemote,
  UsageStatistics,
  VoiceSessionRecord,
} from "./types";

export const api = {
  getSettings: () => invoke<AppSettings>("get_settings"),
  saveSettings: (settings: AppSettings) => invoke<void>("save_settings", { settings }),
  getBleSnapshot: () => invoke<BleSnapshot>("get_ble_snapshot"),
  listPairedRemotes: () => invoke<PairedRemote[]>("list_paired_remotes"),
  connectRemote: (deviceId: string, name: string) =>
    invoke<void>("connect_remote", { deviceId, name }),
  disconnectRemote: () => invoke<void>("disconnect_remote"),
  reconnectRemote: () => invoke<void>("reconnect_remote"),
  listAudioEndpoints: () => invoke<AudioEndpoint[]>("list_audio_endpoints"),
  selectAudioEndpoint: (id: string, name: string) =>
    invoke<void>("select_audio_endpoint", { id, name }),
  getStatistics: () => invoke<UsageStatistics>("get_statistics"),
  getHistory: (limit?: number) => invoke<VoiceSessionRecord[]>("get_history", { limit }),
  clearHistory: () => invoke<void>("clear_history"),
  bindProcessToProfile: (process: string, profileId: string) =>
    invoke<void>("bind_process_to_profile", { process, profileId }),
  unbindProcess: (process: string) => invoke<void>("unbind_process", { process }),
  getForegroundProcess: () => invoke<string | null>("get_foreground_process"),
  simulateButton: (button: string, gesture: string) =>
    invoke<void>("simulate_button", { button, gesture }),
  simulateVoice: (durationMs?: number) => invoke<void>("simulate_voice", { durationMs }),
  runDiagnostics: () => invoke<DiagnosticItem[]>("run_diagnostics"),
  openLogsFolder: () => invoke<void>("open_logs_folder"),
};
