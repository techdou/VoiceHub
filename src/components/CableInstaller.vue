<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { api } from "../api";
import { useI18n, type I18nKey } from "../i18n";
import type { CableStatus } from "../types";

/// VB-CABLE 一键安装：按钮 + 进度轮询（1s）+ 完成/失败状态。
/// 安装完成（端点或驱动服务出现）后 emit installed，由页面接续选端点。

const emit = defineEmits<{ installed: [] }>();
const { t } = useI18n();

const status = ref<CableStatus | null>(null);
const starting = ref(false);
let timer: number | null = null;

async function refresh() {
  const next = await api.checkVirtualCable();
  const wasInstalled = status.value?.endpointPresent || status.value?.servicePresent;
  status.value = next;
  const nowInstalled = next.endpointPresent || next.servicePresent;
  if (wasInstalled === false && nowInstalled && next.installState?.state === "installed") {
    stopPolling();
    emit("installed");
  }
  if (!next.busy && next.installState && !["installed"].includes(next.installState.state)) {
    // 终态（成功已处理，失败停在原地展示）。
    stopPolling();
  }
}

function startPolling() {
  stopPolling();
  timer = window.setInterval(refresh, 1000);
}

function stopPolling() {
  if (timer !== null) {
    window.clearInterval(timer);
    timer = null;
  }
}

async function install() {
  starting.value = true;
  try {
    await api.startCableInstall();
    startPolling();
  } finally {
    starting.value = false;
  }
}

onMounted(refresh);
onBeforeUnmount(stopPolling);

const installed = computed(
  () => status.value?.endpointPresent || status.value?.servicePresent,
);

const busy = computed(() => (status.value?.busy ?? false) || starting.value);

function stateText(state: string | undefined): string {
  if (!state) return "";
  return t(`cable.state.${state}` as I18nKey);
}
</script>

<template>
  <div v-if="!installed" class="cable-install">
    <div class="row">
      <button class="btn primary" :disabled="busy" @click="install">
        {{ busy ? t("cable.installing") : t("cable.install") }}
      </button>
      <span v-if="status?.installState && status.installState.state !== 'not_started'" class="cable-state">
        {{ stateText(status.installState.state) }}
      </span>
    </div>
    <p class="hint" style="margin: 8px 0 0">{{ t("cable.source_note") }}</p>
  </div>
  <div v-else class="row">
    <span class="status-dot ok"></span>
    <span style="font-size: 12.5px">{{ t("cable.done") }}</span>
  </div>
</template>

<style scoped>
.cable-install .cable-state {
  font-size: 12.5px;
  color: var(--text-secondary);
}
</style>
