<script setup lang="ts">
import { computed, onMounted, ref, watchEffect } from "vue";
import { listen } from "@tauri-apps/api/event";
import { openUrl, openPath } from "@tauri-apps/plugin-opener";
import { api } from "../api";
import { useI18n } from "../i18n";
import CableInstaller from "../components/CableInstaller.vue";
import type { AppSettings, AudioEndpoint, BleSnapshot, PairedRemote, UiEvent } from "../types";

const props = defineProps<{
  settings: AppSettings | null;
  bleSnapshot: BleSnapshot | null;
  saveTick: number;
}>();

const emit = defineEmits<{ "update-settings": [settings: AppSettings] }>();
const { t } = useI18n();

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
    await pickEndpoint(cable.id, cable.name);
  }
}

async function pickEndpoint(id: string, name: string) {
  if (!props.settings) return;
  await api.selectAudioEndpoint(id, name);
  emit("update-settings", { ...props.settings, audioEndpointName: name });
}

function pickProvider(kind: AppSettings["provider"]["kind"]) {
  if (!props.settings) return;
  emit("update-settings", {
    ...props.settings,
    provider: { ...props.settings.provider, kind },
  });
}

// 拖动只改草稿值（不写盘），松手 @change 才保存。
const gainDraft = ref(0);
watchEffect(() => {
  gainDraft.value = props.settings?.gainDb ?? 0;
});

function commitGain() {
  if (!props.settings) return;
  emit("update-settings", { ...props.settings, gainDb: gainDraft.value });
}

function setLanguage(language: AppSettings["language"]) {
  if (!props.settings) return;
  emit("update-settings", { ...props.settings, language });
}

function setTheme(theme: AppSettings["theme"]) {
  if (!props.settings) return;
  emit("update-settings", { ...props.settings, theme });
}

function setAutostart(enabled: boolean) {
  if (!props.settings) return;
  emit("update-settings", { ...props.settings, launchAtLogin: enabled });
}

function setCustomMode(mode: "toggle" | "hold") {
  if (!props.settings) return;
  emit("update-settings", {
    ...props.settings,
    provider: { ...props.settings.provider, customMode: mode },
  });
}

function phaseLabel(phase: string | undefined): string {
  if (!phase) return "—";
  return t(`connection.phase.${phase}` as never);
}

onMounted(async () => {
  await Promise.all([refreshRemotes(), refreshEndpoints()]);
  selectedRemoteId.value = props.settings?.pairedDeviceId ?? "";
});

const recording = ref(false);
const voiceLevel = ref(0);

onMounted(async () => {
  await listen<UiEvent>("bridge://event", (event) => {
    if (event.payload.type === "VoiceState") {
      recording.value = event.payload.recording;
      voiceLevel.value = event.payload.level;
    }
  });
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
  <div class="page" v-if="settings">
    <h1>{{ t("connection.title") }}</h1>
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
          <button class="btn" v-else-if="settings.pairedDeviceId" @click="api.reconnectRemote()">
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
      <p class="hint">{{ t("connection.remote.none") }}</p>
      <div class="row" style="margin-bottom: 10px">
        <button class="btn" @click="refreshRemotes">{{ t("common.refresh") }}</button>
        <button class="btn" @click="openPath('ms-settings:bluetooth')">
          {{ t("onboarding.open_settings") }}
        </button>
      </div>
      <div style="display: grid; gap: 6px">
        <button
          v-for="remote in remotes"
          :key="remote.id"
          class="picker-item"
          :class="{ current: settings.pairedDeviceId === remote.id }"
          :disabled="busy"
          @click="connect(remote.id, remote.name)"
        >
          <span>{{ remote.name }}</span>
          <span v-if="settings.pairedDeviceId === remote.id">✓</span>
        </button>
        <div v-if="!remotes.length" class="empty">{{ t("common.empty") }}</div>
      </div>
    </section>

    <section class="card">
      <h3>{{ t("connection.audio.title") }}</h3>
      <p class="hint">{{ t("connection.audio.hint") }}</p>
      <div class="row" style="margin-bottom: 10px">
        <select
          v-if="endpoints.length"
          :value="settings.audioEndpointName"
          @change="
            const target = endpoints.find((e) => e.name === ($event.target as HTMLSelectElement).value);
            if (target) pickEndpoint(target.id, target.name);
          "
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
          :class="{ current: settings.provider.kind === option.id }"
          @click="pickProvider(option.id)"
        >
          <span>{{ t(`connection.provider.${option.id}` as never) }}</span>
        </button>
      </div>
      <p v-if="settings.provider.kind === 'sayit'" class="hint">
        {{ t("connection.provider.sayit_hint") }}
      </p>
      <p v-if="settings.provider.kind === 'we_type'" class="hint">
        {{ t("connection.provider.we_type_hint") }}
      </p>
      <div v-if="settings.provider.kind === 'custom'" class="setting-row">
        <div>
          <div class="label">{{ t("connection.provider.mode.toggle") }} / {{ t("connection.provider.mode.hold") }}</div>
        </div>
        <div class="row">
          <button
            class="btn"
            :class="{ primary: settings.provider.customMode === 'toggle' }"
            @click="setCustomMode('toggle')"
          >
            {{ t("connection.provider.mode.toggle") }}
          </button>
          <button
            class="btn"
            :class="{ primary: settings.provider.customMode === 'hold' }"
            @click="setCustomMode('hold')"
          >
            {{ t("connection.provider.mode.hold") }}
          </button>
        </div>
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
        <select :value="settings.language" @change="setLanguage(($event.target as HTMLSelectElement).value as AppSettings['language'])">
          <option value="system">{{ t("lang.system") }}</option>
          <option value="zh_cn">{{ t("lang.zh") }}</option>
          <option value="english">{{ t("lang.en") }}</option>
        </select>
      </div>
      <div class="setting-row">
        <div class="label">{{ t("settings.theme") }}</div>
        <select :value="settings.theme" @change="setTheme(($event.target as HTMLSelectElement).value as AppSettings['theme'])">
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
          :class="{ on: settings.launchAtLogin }"
          @click="setAutostart(!settings.launchAtLogin)"
        ></button>
      </div>
    </section>
  </div>
  <div v-else class="page">
    <p class="empty">{{ t("common.loading") }}</p>
  </div>
</template>
