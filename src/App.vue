<script setup lang="ts">
import { computed, onMounted, onBeforeUnmount, ref, shallowRef, watch } from "vue";
import { listen } from "@tauri-apps/api/event";
import { openUrl, openPath } from "@tauri-apps/plugin-opener";
import { api } from "./api";
import { resolveLocale, setLocale, useI18n } from "./i18n";
import type { AppSettings, BleSnapshot, UiEvent } from "./types";
import Sidebar from "./components/Sidebar.vue";
import ConnectionPage from "./pages/ConnectionPage.vue";
import ButtonsPage from "./pages/ButtonsPage.vue";
import StatsPage from "./pages/StatsPage.vue";
import HistoryPage from "./pages/HistoryPage.vue";
import DiagnosticsPage from "./pages/DiagnosticsPage.vue";
import AboutPage from "./pages/AboutPage.vue";
import Onboarding from "./components/Onboarding.vue";

const { t } = useI18n();
const version = __APP_VERSION__;
const props = defineProps<{ embedded?: boolean; initialPage?: string; locale?: "zh" | "en" }>();

const page = ref<"connection" | "buttons" | "stats" | "history" | "diagnostics" | "about">("connection");
type PageId = (typeof page)["value"];
watch(() => props.initialPage, (value) => {
  if (value && ["connection", "buttons", "stats", "history", "diagnostics", "about"].includes(value)) page.value = value as PageId;
}, { immediate: true });
const settings = shallowRef<AppSettings | null>(null);
const bleSnapshot = ref<BleSnapshot | null>(null);
const recording = ref(false);
const voiceLevel = ref(0);
const activeButtons = ref(new Set<string>());
const showOnboarding = ref(false);
const saveState = ref<"idle" | "saving" | "saved" | "error">("idle");
const saveError = ref("");
let saveStateTimer: number | undefined;

function locale() {
  return navigator.language || "en-US";
}

async function reloadSettings() {
  settings.value = await api.getSettings();
  if (!props.embedded) setLocale(resolveLocale(settings.value.language, locale()));
  if (settings.value && !settings.value.onboardingComplete && !props.embedded) {
    showOnboarding.value = true;
  }
}

async function persistSettings(next: AppSettings) {
  settings.value = next;
  saveState.value = "saving";
  saveError.value = "";
  window.clearTimeout(saveStateTimer);
  try {
    await api.saveSettings(next);
    saveState.value = "saved";
    saveStateTimer = window.setTimeout(() => {
      if (saveState.value === "saved") saveState.value = "idle";
    }, 1500);
  } catch (error) {
    // 写盘失败必须可见：UI 已更新而磁盘/后端未同步，重启后设置回退（sayit 序列化坑即由此潜伏）。
    saveState.value = "error";
    saveError.value = String(error);
    console.error("[voicehub] save_settings failed:", error);
    // 乐观更新回滚：回读后端真值，避免 UI 一直显示未落盘的状态。
    try {
      settings.value = await api.getSettings();
    } catch (reloadError) {
      console.error("[voicehub] reload settings after save failure:", reloadError);
    }
  }
}

// 主题即时生效：system → 移除标记；light/dark → 硬控。
watch(
  () => settings.value?.theme,
  (theme) => {
    if (!theme || props.embedded) return;
    if (theme === "system") {
      document.documentElement.removeAttribute("data-theme");
    } else {
      document.documentElement.setAttribute("data-theme", theme);
    }
  },
  { immediate: true },
);

// 语言即时生效。
watch(
  () => settings.value?.language,
  (language) => {
    if (language && !props.embedded) setLocale(resolveLocale(language, navigator.language || "en-US"));
  },
  { immediate: true },
);

watch(() => props.locale, (value) => {
  if (props.embedded && value) setLocale(value);
}, { immediate: true });

async function refreshBle() {
  bleSnapshot.value = await api.getBleSnapshot();
}

let disposed = false;
let unlisten: (() => void) | undefined;
onBeforeUnmount(() => { disposed = true; unlisten?.(); window.clearTimeout(saveStateTimer); });
onMounted(async () => {
  // 先注册监听再取快照：fetch 期间发生的状态变化不会丢；
  // 且任一 fetch 失败不阻断监听注册（否则 UI 永久停在初始态不再响应事件）。
  unlisten = await listen<UiEvent>("bridge://event", (event) => {
    const payload = event.payload;
    switch (payload.type) {
      case "BleState":
        bleSnapshot.value = payload.snapshot;
        // 断开时清空按住态（防止遥控器断连导致高亮卡死）。
        if (payload.snapshot.phase !== "ready") {
          activeButtons.value = new Set();
        }
        break;
      case "VoiceState":
        recording.value = payload.recording;
        voiceLevel.value = payload.level;
        break;
      case "Battery":
        // 电量实时推送：BLE 只发 Battery 事件、快照未必重推，原地更新避免 UI 显示旧电量。
        if (bleSnapshot.value) {
          bleSnapshot.value = { ...bleSnapshot.value, batteryPercent: payload.percent };
        }
        break;
      case "ButtonActivity":
        {
          const next = new Set(activeButtons.value);
          if (payload.pressed) {
            next.add(payload.button);
          } else {
            next.delete(payload.button);
          }
          activeButtons.value = next;
        }
        break;
      case "ShowSettings":
        showOnboarding.value = false;
        break;
      default:
        break;
    }
  });
  if (disposed) unlisten();
  try {
    await reloadSettings();
  } catch (error) {
    console.error("[voicehub] load settings failed:", error);
  }
  try {
    await refreshBle();
  } catch (error) {
    console.error("[voicehub] ble snapshot failed:", error);
  }
});
</script>

<template>
  <div class="app-shell">
    <Sidebar
      v-if="!embedded"
      :page="page"
      :recording="recording"
      :ble-phase="bleSnapshot?.phase ?? null"
      @navigate="(target) => (page = target as PageId)"
    />
    <main class="main">
      <ConnectionPage
        v-if="page === 'connection'"
        :settings="settings"
        :ble-snapshot="bleSnapshot"
        :save-state="saveState"
        :save-error="saveError"
        @update-settings="persistSettings"
        @sync-settings="(next) => (settings = next)"
      />
      <ButtonsPage
        v-else-if="page === 'buttons'"
        :settings="settings"
        :save-state="saveState"
        :save-error="saveError"
        :active-buttons="activeButtons"
        :voice-active="recording"
        @update-settings="persistSettings"
      />
      <StatsPage v-else-if="page === 'stats'" />
      <HistoryPage v-else-if="page === 'history'" />
      <DiagnosticsPage v-else-if="page === 'diagnostics'" />
      <AboutPage v-else-if="page === 'about'" :version="version" @open="openUrl" />
    </main>
    <Onboarding
      v-if="showOnboarding && settings"
      :settings="settings"
      @update-settings="persistSettings"
      @finish="
        showOnboarding = false;
        page = 'connection';
      "
    />
  </div>
</template>
