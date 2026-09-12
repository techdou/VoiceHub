<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";
import { formatDuration, useI18n } from "../i18n";
import type { UiEvent, VoiceSessionRecord } from "../types";

const { t } = useI18n();
const records = ref<VoiceSessionRecord[]>([]);
let unlisten: (() => void) | undefined;
// mounted 与 unmount 竞态：listen 落定前组件被卸载则清理函数无人调（App.vue 同款）。
let disposed = false;

onMounted(async () => {
  unlisten = await listen<UiEvent>("bridge://event", (event) => {
    if (event.payload.type === "VoiceState" && !event.payload.recording) refresh();
  });
  if (disposed) unlisten();
  try {
    await refresh();
  } catch (error) {
    console.error("[voicehub] load history failed:", error);
  }
});

onBeforeUnmount(() => {
  unlisten?.();
  unlisten = undefined;
});

async function refresh() {
  records.value = await api.getHistory(200);
}

function formatTime(ms: number): string {
  const date = new Date(ms);
  return `${date.getMonth() + 1}-${String(date.getDate()).padStart(2, "0")} ${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

async function clearAll() {
  await api.clearHistory();
  await refresh();
}
</script>

<template>
  <div class="page">
    <div class="row between" style="margin-bottom: 4px">
      <h1 style="margin: 0">{{ t("history.title") }}</h1>
      <div class="row">
        <button class="btn" @click="refresh">{{ t("common.refresh") }}</button>
        <button v-if="records.length" class="btn danger" @click="clearAll">
          {{ t("history.clear") }}
        </button>
      </div>
    </div>
    <p class="page-sub">{{ t("history.privacy") }}</p>

    <div v-if="records.length" class="history-list">
      <div v-for="record in records" :key="`${record.startedAtMs}-${record.durationMs}`" class="history-item">
        <div>
          <strong>{{ formatTime(record.startedAtMs) }}</strong>
          <span class="meta">
            <span>{{ t("history.duration") }}：{{ formatDuration(record.durationMs) }}</span>
            <span v-if="record.foregroundProcess">
              {{ t("history.app") }}：{{ record.foregroundProcess }}
            </span>
          </span>
        </div>
      </div>
    </div>
    <div v-else class="card">
      <div class="empty">{{ t("history.empty") }}</div>
    </div>
  </div>
</template>
