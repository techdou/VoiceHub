<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch, watchEffect } from "vue";
import { listen } from "@tauri-apps/api/event";
import { openUrl, openPath } from "@tauri-apps/plugin-opener";
import { api } from "../api";
import { useI18n } from "../i18n";
import CableInstaller from "../components/CableInstaller.vue";
import SaveBadge from "../components/SaveBadge.vue";
import type { AppSettings, AudioEndpoint, BleSnapshot, PairedRemote, UiEvent } from "../types";

const props = defineProps<{
  settings: AppSettings | null;
  bleSnapshot: BleSnapshot | null;
  saveState: "idle" | "saving" | "saved" | "error";
  saveError: string;
}>();

const emit = defineEmits<{ "update-settings": [settings: AppSettings] }>();
const { t } = useI18n();

// 草稿模式：改动先落 draft，点「保存」才真实生效+写盘。
// 背景：音频端点保存时会真实打开设备（可能失败），必须让用户明确地
// 「保存 → 看到成功/失败」，而不是改完静默丢失（2026-09-06 事故）。
const draft = ref<AppSettings | null>(null);
watch(
  () => props.settings,
  (next) => {
    draft.value = next ? structuredClone(next) : null;
  },
  { immediate: true },
);

const dirty = computed(
  () =>
    !!draft.value &&
    !!props.settings &&
    JSON.stringify(draft.value) !== JSON.stringify(props.settings),
);

const saving = ref(false);
const endpointError = ref("");

async function saveAll() {
  const next = draft.value;
  if (!next || !props.settings || saving.value) return;
  saving.value = true;
  endpointError.value = "";
  try {
    // 端点变化优先走真实打开（失败即中止，其余设置也不落盘，用户可重试）。
    if (next.audioEndpointName !== props.settings.audioEndpointName) {
      const target = endpoints.value.find((e) => e.name === next.audioEndpointName);
      if (!target) {
        endpointError.value = `端点「${next.audioEndpointName}」不在列表中，请先刷新`;
        return;
      }
      await api.selectAudioEndpoint(target.id, target.name);
    }
    // 其余字段统一写盘（端点名已由 selectAudioEndpoint 保存，此处全量覆盖不冲突）。
    emit("update-settings", next);
  } catch (error) {
    // 端点打不开（独占占用等）：原地显示原因，草稿保留，UI 不回弹。
    endpointError.value = String(error);
  } finally {
    saving.value = false;
  }
}

const remotes = ref<PairedRemote[]>([]);
const endpoints = ref<AudioEndpoint[]>([]);
const selectedRemoteId = ref("");
const busy = ref(false);

async function refreshRemotes() {
  remotes.value = await api.listPairedRemotes();
}

async function refreshEndpoints() {
  endpoints.value = await api.listAudioEndpoints();
}

async function connect(id: string, name: string) {
  selectedRemoteId.value = id;
  busy.value = true;
  try {
    await api.connectRemote(id, name);
  } finally {
    busy.value = false;
  }
}

async function onCableInstalled() {
  await refreshEndpoints();
  const cable = endpoints.value.find((e) => e.isVirtualCableCandidate);
  if (cable) {
    // 安装完成的自动选择是即时动作：真实打开 + 立即写盘。
    try {
      await api.selectAudioEndpoint(cable.id, cable.name);
      endpointError.value = "";
      if (draft.value) draft.value.audioEndpointName = cable.name;
    } catch (error) {
      endpointError.value = String(error);
    }
  }
}

function pickProvider(kind: AppSettings["provider"]["kind"]) {
  if (!draft.value) return;
  draft.value = {
    ...draft.value,
    provider: { ...draft.value.provider, kind },
  };
}

function setEndpointName(name: string) {
  if (!draft.value) return;
  draft.value = { ...draft.value, audioEndpointName: name };
}

// 拖动只改草稿值（不写盘），松手 @change 才进 draft。
const gainDraft = ref(0);
watchEffect(() => {
  gainDraft.value = draft.value?.gainDb ?? 0;
});

function commitGain() {
  if (!draft.value) return;
  draft.value = { ...draft.value, gainDb: gainDraft.value };
}

function setLanguage(language: AppSettings["language"]) {
  if (!draft.value) return;
  draft.value = { ...draft.value, language };
}

function setTheme(theme: AppSettings["theme"]) {
  if (!draft.value) return;
  draft.value = { ...draft.value, theme };
}

function setAutostart(enabled: boolean) {
  if (!draft.value) return;
  draft.value = { ...draft.value, launchAtLogin: enabled };
}

function setSayItTrigger(vk: number, modifiers: number) {
  if (!draft.value) return;
  draft.value = {
    ...draft.value,
    provider: { ...draft.value.provider, sayitVk: vk, sayitModifiers: modifiers },
  };
}

/** 下拉值编码 "vk:modifiers"（0 视为默认右 Alt 单键）。 */
function sayItTriggerValue(): string {
  const provider = draft.value?.provider;
  if (!provider) return "165:0";
  return `${provider.sayitVk || 0xa5}:${provider.sayitModifiers || 0}`;
}

function setVoiceExtend(enabled: boolean) {
  if (!draft.value) return;
  draft.value = { ...draft.value, experimentalVoiceExtend: enabled };
}

function setSayItMode(mode: "toggle" | "hold") {
  if (!draft.value) return;
  draft.value = {
    ...draft.value,
    provider: { ...draft.value.provider, customMode: mode },
  };
}

function setCustomMode(mode: "toggle" | "hold") {
  if (!draft.value) return;
  draft.value = {
    ...draft.value,
    provider: { ...draft.value.provider, customMode: mode },
  };
}

// Custom provider 触发键录制（keydown 捕获 → Windows VK；
// 与 ActionPicker 的录制器同一套 MOD 约定：Alt=1 Ctrl=2 Shift=4 Win=8）。
const recordingCustomKey = ref(false);

function customKeyLabel(vk: number, modifiers: number): string {
  if (!vk) return t("connection.provider.custom_key_unset");
  const special: Record<number, string> = {
    0x08: "Backspace", 0x09: "Tab", 0x0d: "Enter", 0x1b: "Esc", 0x20: "Space",
    0x21: "PgUp", 0x22: "PgDn", 0x23: "End", 0x24: "Home",
    0x25: "←", 0x26: "↑", 0x27: "→", 0x28: "↓", 0x2d: "Insert", 0x2e: "Delete",
    0xa0: "Shift", 0xa1: "RShift", 0xa2: "Ctrl", 0xa3: "RCtrl",
    0xa4: "Alt", 0xa5: "RAlt", 0x5b: "Win",
  };
  const base =
    special[vk] ??
    (vk >= 0x70 && vk <= 0x7b
      ? `F${vk - 0x6f}`
      : (vk >= 0x30 && vk <= 0x39) || (vk >= 0x41 && vk <= 0x5a)
        ? String.fromCharCode(vk)
        : `0x${vk.toString(16).toUpperCase()}`);
  const parts = [
    modifiers & 2 ? "Ctrl" : "",
    modifiers & 4 ? "Shift" : "",
    modifiers & 1 ? "Alt" : "",
    modifiers & 8 ? "Win" : "",
  ].filter(Boolean);
  return [...parts, base].join("+");
}

function onCustomKeyCapture(event: KeyboardEvent) {
  if (!recordingCustomKey.value) return;
  event.preventDefault();
  event.stopPropagation();
  // 纯修饰键按下不算完成（等主键）。
  if (["Control", "Shift", "Alt", "Meta"].includes(event.key)) return;
  const vk = event.which || event.keyCode;
  if (!vk || !draft.value) return;
  let modifiers = 0;
  if (event.ctrlKey) modifiers |= 2;
  if (event.shiftKey) modifiers |= 4;
  if (event.altKey) modifiers |= 1;
  if (event.metaKey) modifiers |= 8;
  draft.value = {
    ...draft.value,
    provider: { ...draft.value.provider, customVk: vk, customModifiers: modifiers },
  };
  recordingCustomKey.value = false;
}

onMounted(() => window.addEventListener("keydown", onCustomKeyCapture, true));
onBeforeUnmount(() => window.removeEventListener("keydown", onCustomKeyCapture, true));

function phaseLabel(phase: string | undefined): string {
  if (!phase) return "—";
  return t(`connection.phase.${phase}` as never);
}

function isConnectedRemote(id: string): boolean {
  return props.settings?.pairedDeviceId === id && props.bleSnapshot?.phase === "ready";
}

onMounted(async () => {
  await Promise.all([refreshRemotes(), refreshEndpoints()]);
  selectedRemoteId.value = props.settings?.pairedDeviceId ?? "";
});

const recording = ref(false);
const voiceLevel = ref(0);
let unlisten: (() => void) | undefined;

onMounted(async () => {
  unlisten = await listen<UiEvent>("bridge://event", (event) => {
    if (event.payload.type === "VoiceState") {
      recording.value = event.payload.recording;
      voiceLevel.value = event.payload.level;
    }
  });
});

onBeforeUnmount(() => {
  unlisten?.();
  unlisten = undefined;
});

const cableCandidatePresent = computed(() =>
  endpoints.value.some((e) => e.isVirtualCableCandidate),
);

const providerOptions = [
  { id: "sayit", hint: true },
  { id: "we_type", hint: true },
  { id: "doubao" },
  { id: "win_h" },
  { id: "custom" },
  { id: "none" },
] as const;
</script>

<template>
  <div class="page" v-if="draft">
    <div class="row between" style="align-items: baseline">
      <h1>{{ t("connection.title") }}</h1>
      <div class="row" style="gap: 10px; align-items: center">
        <span v-if="dirty" class="dirty-mark">{{ t("connection.unsaved_changes") }}</span>
        <button
          class="btn"
          :class="{ primary: dirty }"
          :disabled="!dirty || saving"
          @click="saveAll"
        >
          {{ saving ? t("common.saving") : t("common.save") }}
        </button>
        <SaveBadge :state="saveState" :error="saveError" />
      </div>
    </div>
    <p class="page-sub">{{ t("app.tagline") }}</p>

    <section class="card">
      <h3>{{ t("connection.status") }}</h3>
      <div class="row between">
        <div class="row">
          <span
            class="status-dot"
            :class="
              bleSnapshot?.phase === 'ready'
                ? 'ok'
                : bleSnapshot?.phase === 'failed'
                  ? 'fail'
                  : bleSnapshot?.phase === 'stopped'
                    ? 'info'
                    : 'warn'
            "
          ></span>
          <strong>{{ phaseLabel(bleSnapshot?.phase) }}</strong>
          <span v-if="bleSnapshot?.remoteName" class="badge">{{ bleSnapshot.remoteName }}</span>
          <span v-if="bleSnapshot?.remoteModel" class="badge">{{ bleSnapshot.remoteModel }}</span>
          <span v-if="bleSnapshot?.batteryPercent != null" class="badge">
            {{ t("connection.battery") }} {{ bleSnapshot.batteryPercent }}%
          </span>
        </div>
        <div class="row">
          <button class="btn" v-if="bleSnapshot?.phase === 'ready'" @click="api.disconnectRemote()">
            {{ t("common.disconnect") }}
          </button>
          <button class="btn" v-else-if="draft.pairedDeviceId" @click="api.reconnectRemote()">
            {{ t("common.retry") }}
          </button>
        </div>
      </div>
      <div v-if="recording" class="row" style="margin-top: 10px">
        <span class="pulse-dot recording"></span>
        <span style="font-size: 12px; color: var(--text-secondary)">语音中</span>
        <div class="level-bar">
          <div class="fill" :style="{ width: `${Math.min(100, voiceLevel * 140)}%` }"></div>
        </div>
      </div>
      <p v-if="bleSnapshot?.lastError" class="hint" style="margin-top: 8px; color: var(--fail)">
        {{ bleSnapshot.lastError }}
      </p>
    </section>

    <section class="card">
      <h3>{{ t("connection.remote.select") }}</h3>
      <p v-if="!remotes.length" class="hint">{{ t("connection.remote.none") }}</p>
      <div class="row" style="margin-bottom: 10px">
        <button class="btn" @click="refreshRemotes">{{ t("common.refresh") }}</button>
        <button class="btn" @click="openPath('ms-settings:bluetooth')">
          {{ t("onboarding.open_settings") }}
        </button>
      </div>
      <div class="device-list">
        <button
          v-for="remote in remotes"
          :key="remote.id"
          class="device-item"
          :class="{ connected: isConnectedRemote(remote.id) }"
          :disabled="busy"
          @click="connect(remote.id, remote.name)"
        >
          <span class="status-dot" :class="isConnectedRemote(remote.id) ? 'ok' : 'off'"></span>
          <span class="device-name">{{ remote.name }}</span>
          <template v-if="isConnectedRemote(remote.id)">
            <span v-if="bleSnapshot?.batteryPercent != null" class="badge">
              {{ t("connection.battery") }} {{ bleSnapshot.batteryPercent }}%
            </span>
            <span class="device-connected">{{ t("connection.connected") }}</span>
          </template>
        </button>
        <div v-if="!remotes.length" class="empty">{{ t("common.empty") }}</div>
      </div>
    </section>

    <section class="card">
      <h3>{{ t("connection.audio.title") }}</h3>
      <p class="hint">{{ t("connection.audio.hint") }}</p>
      <p v-if="endpointError" class="hint" style="color: var(--fail)">{{ endpointError }}</p>
      <div class="row" style="margin-bottom: 10px">
        <select
          v-if="endpoints.length"
          :value="draft.audioEndpointName"
          @change="setEndpointName(($event.target as HTMLSelectElement).value)"
        >
          <option v-for="endpoint in endpoints" :key="endpoint.id" :value="endpoint.name">
            {{ endpoint.name }}{{ endpoint.isVirtualCableCandidate ? "  · CABLE" : "" }}
          </option>
        </select>
        <button class="btn" @click="refreshEndpoints">{{ t("common.refresh") }}</button>
        <span class="spacer"></span>
        <button class="btn subtle" @click="openUrl('https://vb-audio.com/Cable/')">
          {{ t("connection.audio.install_vbcable") }}
        </button>
      </div>
      <CableInstaller
        v-if="!cableCandidatePresent"
        @installed="onCableInstalled"
      />
      <div class="row" style="margin-top: 10px">
        <button class="btn" @click="api.simulateVoice(2000)">{{ t("connection.voice_test") }}</button>
        <span class="hint" style="margin: 0">{{ t("sim.voice_hint") }}</span>
      </div>
    </section>

    <section class="card">
      <h3>{{ t("connection.provider.title") }}</h3>
      <div style="display: grid; gap: 6px; margin-bottom: 12px">
        <button
          v-for="option in providerOptions"
          :key="option.id"
          class="picker-item"
          :class="{ current: draft.provider.kind === option.id }"
          @click="pickProvider(option.id)"
        >
          <span>{{ t(`connection.provider.${option.id}` as never) }}</span>
        </button>
      </div>
      <p v-if="draft.provider.kind === 'sayit'" class="hint">
        {{ t("connection.provider.sayit_hint") }}
      </p>
      <div v-if="draft.provider.kind === 'sayit'" class="setting-row">
        <div class="label">{{ t("connection.provider.sayit_mode") }}</div>
        <div class="row">
          <button
            class="btn"
            :class="{ primary: draft.provider.customMode === 'toggle' }"
            @click="setSayItMode('toggle')"
          >
            {{ t("connection.provider.sayit_mode.toggle") }}
          </button>
          <button
            class="btn"
            :class="{ primary: draft.provider.customMode === 'hold' }"
            @click="setSayItMode('hold')"
          >
            {{ t("connection.provider.sayit_mode.hold") }}
          </button>
        </div>
      </div>
      <div v-if="draft.provider.kind === 'sayit'" class="setting-row">
        <div class="label">{{ t("connection.provider.sayit_key") }}</div>
        <select
          :value="sayItTriggerValue()"
          @change="
            const [vk, mods] = ($event.target as HTMLSelectElement).value.split(':').map(Number);
            setSayItTrigger(vk, mods);
          "
        >
          <option value="72:3">{{ t("connection.provider.sayit_key.combo") }}</option>
          <option value="165:0">{{ t("connection.provider.sayit_key.ralt") }}</option>
          <option value="163:0">{{ t("connection.provider.sayit_key.rctrl") }}</option>
        </select>
      </div>
      <p v-if="draft.provider.kind === 'we_type'" class="hint">
        {{ t("connection.provider.we_type_hint") }}
      </p>
      <div v-if="draft.provider.kind === 'custom'" class="setting-row">
        <div>
          <div class="label">{{ t("connection.provider.custom_key") }}</div>
          <div class="desc">
            {{ customKeyLabel(draft.provider.customVk, draft.provider.customModifiers) }}
          </div>
        </div>
        <button class="btn" @click="recordingCustomKey = !recordingCustomKey">
          {{ recordingCustomKey ? t("connection.provider.custom_key_listening") : t("connection.provider.custom_key_record") }}
        </button>
      </div>
      <p v-if="draft.provider.kind === 'custom' && recordingCustomKey" class="hint">
        {{ t("connection.provider.custom_key_listening") }}
      </p>
      <div v-if="draft.provider.kind === 'custom'" class="setting-row">
        <div>
          <div class="label">{{ t("connection.provider.mode.toggle") }} / {{ t("connection.provider.mode.hold") }}</div>
        </div>
        <div class="row">
          <button
            class="btn"
            :class="{ primary: draft.provider.customMode === 'toggle' }"
            @click="setCustomMode('toggle')"
          >
            {{ t("connection.provider.mode.toggle") }}
          </button>
          <button
            class="btn"
            :class="{ primary: draft.provider.customMode === 'hold' }"
            @click="setCustomMode('hold')"
          >
            {{ t("connection.provider.mode.hold") }}
          </button>
        </div>
      </div>
    </section>

    <section class="card">
      <div class="setting-row" style="padding-top: 0">
        <div>
          <div class="label">{{ t("voice.extend.title") }}</div>
          <div class="desc">{{ t("voice.extend.hint") }}</div>
        </div>
        <button
          class="switch"
          :class="{ on: draft.experimentalVoiceExtend }"
          @click="setVoiceExtend(!draft.experimentalVoiceExtend)"
        ></button>
      </div>
    </section>

    <section class="card">
      <h3>{{ t("connection.gain.title") }}</h3>
      <p class="hint">{{ t("connection.gain.hint") }}</p>
      <div class="row">
        <span>−24</span>
        <input
          type="range"
          min="-24"
          max="24"
          step="1"
          v-model.number="gainDraft"
          @change="commitGain"
        />
        <span>+24</span>
        <strong style="min-width: 48px; text-align: right">{{ gainDraft.toFixed(0) }} dB</strong>
      </div>
    </section>

    <section class="card">
      <h3>{{ t("settings.general") }}</h3>
      <div class="setting-row">
        <div class="label">{{ t("settings.language") }}</div>
        <select :value="draft.language" @change="setLanguage(($event.target as HTMLSelectElement).value as AppSettings['language'])">
          <option value="system">{{ t("lang.system") }}</option>
          <option value="zh_cn">{{ t("lang.zh") }}</option>
          <option value="english">{{ t("lang.en") }}</option>
        </select>
      </div>
      <div class="setting-row">
        <div class="label">{{ t("settings.theme") }}</div>
        <select :value="draft.theme" @change="setTheme(($event.target as HTMLSelectElement).value as AppSettings['theme'])">
          <option value="system">{{ t("theme.system") }}</option>
          <option value="light">{{ t("theme.light") }}</option>
          <option value="dark">{{ t("theme.dark") }}</option>
        </select>
      </div>
      <div class="setting-row">
        <div>
          <div class="label">{{ t("settings.autostart") }}</div>
          <div class="desc">{{ t("settings.autostart.hint") }}</div>
        </div>
        <button
          class="switch"
          :class="{ on: draft.launchAtLogin }"
          @click="setAutostart(!draft.launchAtLogin)"
        ></button>
      </div>
    </section>
  </div>
  <div v-else class="page">
    <p class="empty">{{ t("common.loading") }}</p>
  </div>
</template>

<style scoped>
.dirty-mark {
  font-size: 12px;
  color: var(--warn, #c08a2d);
  white-space: nowrap;
}

.device-list {
  display: grid;
  gap: 8px;
}

.device-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 14px;
  border-radius: 10px;
  border: 1px solid var(--border);
  background: var(--panel-2);
  cursor: pointer;
  font: inherit;
  color: var(--text);
  text-align: left;
  transition: border-color 0.12s, background 0.12s;
}

.device-item:hover {
  border-color: var(--border-strong);
}

.device-item.connected {
  border-color: var(--accent);
  background: var(--accent-soft);
}

.device-item:disabled {
  opacity: 0.6;
  cursor: default;
}

.device-name {
  font-weight: 600;
  font-size: 13.5px;
  flex: 1;
  min-width: 0;
}

.device-connected {
  font-size: 12px;
  color: var(--accent);
  font-weight: 600;
  white-space: nowrap;
}
</style>
