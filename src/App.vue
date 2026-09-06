<script setup lang="ts">
import { computed, onMounted, ref, shallowRef, watch } from "vue";
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
import SimulatorPage from "./pages/SimulatorPage.vue";
import AboutPage from "./pages/AboutPage.vue";
import Onboarding from "./components/Onboarding.vue";

const { t } = useI18n();
const version = __APP_VERSION__;

const page = ref<"connection" | "buttons" | "stats" | "history" | "diagnostics" | "simulator" | "about">("connection");
type PageId = (typeof page)["value"];
const settings = shallowRef<AppSettings | null>(null);
const bleSnapshot = ref<BleSnapshot | null>(null);
const recording = ref(false);
const voiceLevel = ref(0);
const showOnboarding = ref(false);
const saveTick = ref(0);

function locale() {
  return navigator.language || "en-US";
}

async function reloadSettings() {
  settings.value = await api.getSettings();
  setLocale(resolveLocale(settings.value.language, locale()));
  if (settings.value && !settings.value.onboardingComplete) {
    showOnboarding.value = true;
  }
}

async function persistSettings(next: AppSettings) {
  settings.value = next;
  saveTick.value++;
  await api.saveSettings(next);
}

// 主题即时生效：system → 移除标记；light/dark → 硬控。
watch(
  () => settings.value?.theme,
  (theme) => {
    if (!theme) return;
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
    if (language) setLocale(resolveLocale(language, navigator.language || "en-US"));
  },
  { immediate: true },
);

async function refreshBle() {
  bleSnapshot.value = await api.getBleSnapshot();
}

onMounted(async () => {
  await reloadSettings();
  await refreshBle();
  await listen<UiEvent>("bridge://event", (event) => {
    const payload = event.payload;
    switch (payload.type) {
      case "BleState":
        bleSnapshot.value = payload.snapshot;
        break;
      case "VoiceState":
        recording.value = payload.recording;
        voiceLevel.value = payload.level;
        break;
      case "ShowSettings":
        showOnboarding.value = false;
        break;
      default:
        break;
    }
  });
});
</script>

<template>
  <div class="app-shell">
    <Sidebar
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
        :save-tick="saveTick"
        @update-settings="persistSettings"
      />
      <ButtonsPage
        v-else-if="page === 'buttons'"
        :settings="settings"
        :save-tick="saveTick"
        @update-settings="persistSettings"
      />
      <StatsPage v-else-if="page === 'stats'" />
      <HistoryPage v-else-if="page === 'history'" />
      <DiagnosticsPage v-else-if="page === 'diagnostics'" />
      <SimulatorPage v-else-if="page === 'simulator'" />
      <AboutPage v-else-if="page === 'about'" :version="version" @open="openUrl" />
    </main>
    <Onboarding
      v-if="showOnboarding && settings"
      :settings="settings"
      @update-settings="persistSettings"
      @finish="
        showOnboarding = false;
        page = 'simulator';
      "
    />
  </div>
</template>
