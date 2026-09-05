<script setup lang="ts">
import { ref } from "vue";
import { api } from "../api";
import { useI18n } from "../i18n";
import type { DiagnosticItem } from "../types";

const { t } = useI18n();
const items = ref<DiagnosticItem[]>([]);
const running = ref(false);

async function run() {
  running.value = true;
  try {
    items.value = await api.runDiagnostics();
  } finally {
    running.value = false;
  }
}
</script>

<template>
  <div class="page">
    <div class="row between" style="margin-bottom: 4px">
      <h1 style="margin: 0">{{ t("diag.title") }}</h1>
      <div class="row">
        <button class="btn primary" :disabled="running" @click="run">
          {{ t("diag.run") }}
        </button>
        <button class="btn" @click="api.openLogsFolder">{{ t("diag.logs") }}</button>
      </div>
    </div>
    <p class="page-sub">{{ t("connection.audio.hint") }}</p>

    <div v-if="items.length" class="history-list">
      <div v-for="item in items" :key="item.id" class="history-item">
        <div class="row" style="flex: 1">
          <span class="status-dot" :class="item.status"></span>
          <div style="flex: 1">
            <strong>{{ item.title }}</strong>
            <div class="meta">{{ item.detail }}</div>
          </div>
          <span class="badge">{{ t(`diag.status.${item.status}` as never) }}</span>
        </div>
      </div>
    </div>
    <div v-else class="card">
      <div class="empty">{{ t("diag.run") }} →</div>
    </div>
  </div>
</template>
