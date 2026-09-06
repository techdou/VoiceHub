<script setup lang="ts">
import { onMounted, ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";
import { formatDuration, useI18n } from "../i18n";
import type { UiEvent, VoiceSessionRecord } from "../types";

const { t } = useI18n();
const records = ref<VoiceSessionRecord[]>([]);

onMounted(async () => {
  await refresh();
  await listen<UiEvent>("bridge://event", (event) => {
    if (event.payload.type === "VoiceState" && !event.payload.recording) refresh();
  });
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
      <div v-for="(record, index) in records" :key="index" class="history-item">
        <div>
          <strong>{{ formatTime(record.startedAtMs) }}</strong>
          <span class="meta">
            <span>{{ t("history.duration") }}：{{ formatDuration(record.durationMs) }}</span>
            <span v-if="record.foregroundProcess">
              {{ t("history.app") }}：{{ record.foregroundProcess }}
            </span>
            <span>{{ t("history.profile") }}：{{ record.profileName }}</span>
          </span>
        </div>
      </div>
    </div>
    <div v-else class="card">
      <div class="empty">{{ t("history.empty") }}</div>
    </div>
  </div>
</template>
