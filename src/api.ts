import { invoke } from "@tauri-apps/api/core";
import type {
  AppSettings,
  AudioEndpoint,
  CableStatus,
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
  getForegroundProcess: () => invoke<string | null>("get_foreground_process"),
  resetMappingToDefault: () => invoke<void>("reset_mapping_to_default"),
  simulateButton: (button: string, gesture: string) =>
    invoke<void>("simulate_button", { button, gesture }),
  simulateVoice: (durationMs?: number, audioB64?: string) => invoke<void>("simulate_voice", { durationMs, audioB64 }),
  runDiagnostics: () => invoke<DiagnosticItem[]>("run_diagnostics"),
  checkVirtualCable: () => invoke<CableStatus>("check_virtual_cable"),
  startCableInstall: () => invoke<void>("start_cable_install"),
  openLogsFolder: () => invoke<void>("open_logs_folder"),
};
