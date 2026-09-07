<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch, watchEffect } from "vue";
import { listen } from "@tauri-apps/api/event";
import { openUrl, openPath } from "@tauri-apps/plugin-opener";
import { api } from "../api";
import { useI18n } from "../i18n";
import CableInstaller from "../components/CableInstaller.vue";
import PageSkeleton from "../components/PageSkeleton.vue";
import SaveBadge from "../components/SaveBadge.vue";
import type { AppSettings, AudioEndpoint, BleSnapshot, PairedRemote, UiEvent } from "../types";

const props = defineProps<{
  settings: AppSettings | null;
  bleSnapshot: BleSnapshot | null;
  saveState: "idle" | "saving" | "saved" | "error";
  saveError: string;
}>();

const emit = defineEmits<{
  "update-settings": [settings: AppSettings];
  /** 后端已自行落盘的设置回传（连接动作产生）：只更新 App 内存，不再写盘。 */
  "sync-settings": [settings: AppSettings];
}>();
const { t } = useI18n();

// 草稿模式：改动先落 draft，点「保存」才真实生效+写盘。
// 背景：音频端点保存时会真实打开设备（可能失败），必须让用户明确地
// 「保存 → 看到成功/失败」，而不是改完静默丢失（2026-09-06 事故）。
const draft = ref<AppSettings | null>(null);

// dirty 必须先于下面的 watch 声明：watch 是 immediate 的，回调里读 dirty。
const dirty = computed(
  () =>
    !!draft.value &&
    !!props.settings &&
    JSON.stringify(draft.value) !== JSON.stringify(props.settings),
);

watch(
  () => props.settings,
  (next) => {
    if (!next) {
      draft.value = null;
      return;
    }
    if (dirty.value && draft.value) {
      // 用户有未保存编辑（如正在切换 provider）：连接遥控器等后端动作会回传新
      // 设置，此时只合并后端权威字段，其余编辑保留——整体重置会静默丢草稿。
      draft.value = {
        ...draft.value,
        pairedDeviceId: next.pairedDeviceId,
        pairedDeviceName: next.pairedDeviceName,
        onboardingComplete: next.onboardingComplete,
      };
      return;
    }
    // Settings may come back as a nested Vue proxy after saving the draft.
    draft.value = JSON.parse(JSON.stringify(next)) as AppSettings;
  },
  { immediate: true },
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
    if (next.provider.kind !== 'sayit' && next.audioEndpointName !== props.settings.audioEndpointName) {
      const target = endpoints.value.find((e) => e.name === next.audioEndpointName);
      if (!target) {
        endpointError.value = t("connection.audio.endpoint_missing", { name: next.audioEndpointName });
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
const connectError = ref("");

async function refreshRemotes() {
  remotes.value = await api.listPairedRemotes();
}

async function refreshEndpoints() {
  endpoints.value = await api.listAudioEndpoints();
}

async function connect(id: string, name: string) {
  selectedRemoteId.value = id;
  busy.value = true;
  connectError.value = "";
  try {
    await api.connectRemote(id, name);
    // connect_remote 后端已写盘；这里只同步 App 内存（走 sync-settings 不再落盘）。
    emit("sync-settings", await api.getSettings());
  } catch (error) {
    connectError.value = String(error);
    console.error("[voicehub] connect_remote failed:", error);
  } finally {
    busy.value = false;
  }
}

/** 模板按钮的统一动作包装：防连点 + 失败可见，不再裸 invoke。 */
const actionBusy = ref(false);
async function runAction(action: () => Promise<void>) {
  if (actionBusy.value) return;
  actionBusy.value = true;
  connectError.value = "";
  try {
    await action();
  } catch (error) {
    connectError.value = String(error);
    console.error("[voicehub] action failed:", error);
  } finally {
    actionBusy.value = false;
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

function setVoiceExtend(enabled: boolean) {
  if (!draft.value) return;
  draft.value = { ...draft.value, experimentalVoiceExtend: enabled };
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
// mounted 与 unmount 竞态：listen 的 Promise 落定前组件就被卸载的话，
// 清理函数没人调，监听器泄漏（App.vue 同款 disposed 模式）。
let disposed = false;

onMounted(async () => {
  unlisten = await listen<UiEvent>("bridge://event", (event) => {
    if (event.payload.type === "VoiceState") {
      recording.value = event.payload.recording;
      voiceLevel.value = event.payload.level;
    }
  });
  if (disposed) unlisten();
});

onBeforeUnmount(() => {
  disposed = true;
  unlisten?.();
  unlisten = undefined;
});

const cableCandidatePresent = computed(() =>
  endpoints.value.some((e) => e.isVirtualCableCandidate),
);

// "仅音频（不触发工具）"（none）选项已移除：直连引擎开箱即用后无使用价值。
// ProviderKind::None 在 Rust 侧保留仅为反序列化旧配置。
const providerOptions = ["sayit", "we_type", "doubao", "win_h", "custom"] as const;

// ---------- 语音触发（源 × 方式）----------
// 四象限：遥控器语音键 = 固件按住语义（只能按住说话）；免提 / 按住说话可绑到
// 其他遥控器键（注入组合键走系统麦克风）；键盘快捷键独立常驻可用。
// 注意：SayIt 的键盘钩子忽略程序注入的单键，组合键走 RegisterHotKey 才可联动
// ——所以"应用到遥控器"会把引擎侧快捷键一并写成组合键。
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { useKeyRecorder } from "../useKeyRecorder";

const REMOTE_KEY_OPTIONS = [
  "ok", "back", "home", "menu", "up", "down", "left", "right",
  "power", "volume_up", "volume_down", "tv",
] as const;

const handsFreeKey = ref("Ctrl+Alt+H");
const pttComboKey = ref("Ctrl+Alt+J");
const handsFreeRemote = ref<string>("ok");
const pttRemote = ref<string>("back");
const engineHandsFree = ref<string>("");
const enginePtt = ref<string>("");
const triggerBusy = ref("");
const triggerError = ref("");

// sayit 引擎侧当前快捷键（显示用；单键时提示"遥控器不可联动"）。
async function refreshEngineShortcuts() {
  try {
    const [hf, ptt] = await Promise.all([
      tauriInvoke<string>("store_get", { key: "shortcutHandsFree" }),
      tauriInvoke<string>("store_get", { key: "shortcutPTT" }),
    ]);
    engineHandsFree.value = typeof hf === "string" ? hf : "";
    enginePtt.value = typeof ptt === "string" ? ptt : "";
  } catch (error) {
    console.error("[voicehub] read engine shortcuts failed:", error);
  }
}
onMounted(() => { void refreshEngineShortcuts(); });

function isCombo(value: string): boolean {
  return value.includes("+");
}

// 录制组合键（免提 / 按住说话各一个 recorder，共用逻辑）。
const handsFreeRecorder = useKeyRecorder((chord) => {
  handsFreeKey.value = holdLabelOf(chord);
});
const pttRecorder = useKeyRecorder((chord) => {
  pttComboKey.value = holdLabelOf(chord);
});
function holdLabelOf(chord: { vk: number; modifiers: number }): string {
  return handsFreeRecorder.chordLabel(chord.vk, chord.modifiers);
}

/**
 * 一键把链路配置好：引擎侧快捷键写成组合键（store_set + shortcuts_changed），
 * 主应用侧把选定遥控器键映射到该组合键（免提 = 单击注入；按住 = pushToTalk 直通）。
 */
async function applyTrigger(mode: "handsfree" | "ptt") {
  if (!draft.value || !props.settings || saving.value) return;
  triggerBusy.value = mode;
  triggerError.value = "";
  const combo = mode === "handsfree" ? handsFreeKey.value : pttComboKey.value;
  const remoteKey = mode === "handsfree" ? handsFreeRemote.value : pttRemote.value;
  try {
    if (!combo.includes("+")) {
      triggerError.value = t("connection.trigger.needs_combo");
      return;
    }
    // 1) 引擎侧：快捷键 = 组合键（RegisterHotKey 路径，程序注入可触发）。
    await tauriInvoke("store_set", {
      key: mode === "handsfree" ? "shortcutHandsFree" : "shortcutPTT",
      value: combo,
    });
    await tauriInvoke("shortcuts_changed");
    // 2) 主应用侧：遥控器键 → 注入该组合键。
    const vk = combo.split("+").pop()!.trim();
    const vkNum = /^F\d{1,2}$/.test(vk)
      ? 0x6f + Number(vk.slice(1))
      : vk.length === 1 ? vk.toUpperCase().charCodeAt(0) : 0;
    if (!vkNum) {
      triggerError.value = t("connection.trigger.bad_combo");
      return;
    }
    let modBits = 0;
    if (combo.includes("Ctrl")) modBits |= 2;
    if (combo.includes("Shift")) modBits |= 4;
    if (combo.includes("Alt")) modBits |= 1;
    if (combo.includes("Win")) modBits |= 8;
    const label = combo;
    const profiles = JSON.parse(JSON.stringify(draft.value.profiles)) as AppSettings["profiles"];
    const profile = profiles.profiles.find((p) => p.id === profiles.selectedProfileId);
    if (!profile) return;
    const binding = profile.mapping.bindings[remoteKey] ??= {
      single: { kind: "disabled" }, double: { kind: "disabled" }, long: { kind: "disabled" },
    };
    if (mode === "handsfree") {
      binding.single = { kind: "shortcut", vk: vkNum, modifiers: modBits, label };
    } else {
      binding.pushToTalk = { vk: vkNum, modifiers: modBits, label };
      binding.single = { kind: "disabled" };
    }
    draft.value = { ...draft.value, profiles };
    await saveAll();
    await refreshEngineShortcuts();
  } catch (error) {
    triggerError.value = String(error);
  } finally {
    triggerBusy.value = "";
  }
}
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
          <span v-if="bleSnapshot?.phase !== 'ready' && remotes.length" class="hint" style="margin: 0 0 0 4px">
            {{ t("connection.connect_guide") }}
          </span>
          <span v-if="bleSnapshot?.remoteName" class="badge">{{ bleSnapshot.remoteName }}</span>
          <span v-if="bleSnapshot?.remoteModel" class="badge">{{ bleSnapshot.remoteModel }}</span>
          <span v-if="bleSnapshot?.batteryPercent != null" class="badge">
            {{ t("connection.battery") }} {{ bleSnapshot.batteryPercent }}%
          </span>
        </div>
        <div class="row">
          <button
            class="btn"
            v-if="bleSnapshot?.phase === 'ready'"
            :disabled="actionBusy"
            @click="runAction(() => api.disconnectRemote())"
          >
            {{ t("common.disconnect") }}
          </button>
          <button
            class="btn"
            v-else-if="draft.pairedDeviceId"
            :disabled="actionBusy"
            @click="runAction(() => api.reconnectRemote())"
          >
            {{ t("common.retry") }}
          </button>
        </div>
      </div>
      <p v-if="connectError" class="hint" style="margin-top: 8px; color: var(--fail)">
        {{ connectError }}
      </p>
      <div v-if="recording" class="row" style="margin-top: 10px">
        <span class="pulse-dot recording"></span>
        <span style="font-size: 12px; color: var(--text-secondary)">{{ t("connection.recording_badge") }}</span>
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
          <button
            v-else
            class="btn primary"
            :disabled="busy"
            @click.stop="connect(remote.id, remote.name)"
          >
            {{ busy && selectedRemoteId === remote.id ? t("connection.phase.connecting") : t("common.connect") }}
          </button>
        </button>
        <div v-if="!remotes.length" class="empty">{{ t("common.empty") }}</div>
      </div>
    </section>

    <section v-if="draft.provider.kind !== 'sayit'" class="card">
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
        <button class="btn" :disabled="actionBusy" @click="runAction(() => api.simulateVoice(2000))">
          {{ t("connection.voice_test") }}
        </button>
        <span class="hint" style="margin: 0">{{ t("sim.voice_hint") }}</span>
      </div>
    </section>

    <section class="card">
      <h3>{{ t("connection.provider.title") }}</h3>
      <div style="display: grid; gap: 6px; margin-bottom: 12px">
        <button
          v-for="option in providerOptions"
          :key="option"
          class="picker-item"
          :class="{ current: draft.provider.kind === option }"
          @click="pickProvider(option)"
        >
          <span>{{ option === 'sayit' ? t("connection.provider.sayit_embedded") : t(`connection.provider.${option}` as never) }}</span>
        </button>
      </div>
      <div v-if="draft.provider.kind === 'sayit'" class="setting-row">
        <a class="btn" href="#/voice-engine">{{ t("connection.open_engine_settings") }}</a>
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

    <!-- 语音触发：源（遥控器语音键 / 其他遥控器键→麦克风 / 键盘快捷键）× 方式（免提 / 按住说话）。 -->
    <section class="card">
      <h3>{{ t("connection.trigger.title") }}</h3>
      <p class="hint">{{ t("connection.trigger.intro") }}</p>
      <div class="row" style="gap: 8px; margin: 10px 0 4px; flex-wrap: wrap">
        <span class="badge">{{ t("connection.trigger.engine_hf") }}：{{ engineHandsFree || "—" }}{{ engineHandsFree && !isCombo(engineHandsFree) ? t("connection.trigger.single_key_note") : "" }}</span>
        <span class="badge">{{ t("connection.trigger.engine_ptt") }}：{{ enginePtt || "—" }}{{ enginePtt && !isCombo(enginePtt) ? t("connection.trigger.single_key_note") : "" }}</span>
      </div>

      <div class="setting-row" style="border-top: 1px solid var(--border); margin-top: 10px; padding-top: 12px">
        <div>
          <div class="label">{{ t("connection.trigger.hf_title") }}</div>
          <div class="desc">{{ t("connection.trigger.hf_desc") }}</div>
        </div>
      </div>
      <div class="row" style="gap: 8px; flex-wrap: wrap; margin-top: 8px">
        <select v-model="handsFreeRemote" style="min-width: 110px">
          <option v-for="key in REMOTE_KEY_OPTIONS" :key="key" :value="key">{{ t(`buttons.key_names.${key}`) }}</option>
        </select>
        <button class="btn" @click="handsFreeRecorder.recording.value = !handsFreeRecorder.recording.value">
          {{ handsFreeRecorder.recording.value ? t("buttons.action.recording.stop") : handsFreeKey }}
        </button>
        <button class="btn primary" :disabled="!!triggerBusy" @click="applyTrigger('handsfree')">
          {{ triggerBusy === 'handsfree' ? t("common.saving") : t("connection.trigger.apply") }}
        </button>
      </div>

      <div class="setting-row" style="border-top: 1px solid var(--border); margin-top: 12px; padding-top: 12px">
        <div>
          <div class="label">{{ t("connection.trigger.ptt_title") }}</div>
          <div class="desc">{{ t("connection.trigger.ptt_desc") }}</div>
        </div>
      </div>
      <div class="row" style="gap: 8px; flex-wrap: wrap; margin-top: 8px">
        <select v-model="pttRemote" style="min-width: 110px">
          <option v-for="key in REMOTE_KEY_OPTIONS" :key="key" :value="key">{{ t(`buttons.key_names.${key}`) }}</option>
        </select>
        <button class="btn" @click="pttRecorder.recording.value = !pttRecorder.recording.value">
          {{ pttRecorder.recording.value ? t("buttons.action.recording.stop") : pttComboKey }}
        </button>
        <button class="btn primary" :disabled="!!triggerBusy" @click="applyTrigger('ptt')">
          {{ triggerBusy === 'ptt' ? t("common.saving") : t("connection.trigger.apply") }}
        </button>
      </div>
      <p v-if="triggerError" class="hint" style="margin-top: 8px; color: var(--fail)">{{ triggerError }}</p>
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
      <a class="btn" href="#/settings">{{ t("settings.open") }}</a>
    </section>
  </div>
  <PageSkeleton v-else :title="t('connection.title')" />
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
