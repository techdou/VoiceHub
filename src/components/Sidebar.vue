<script setup lang="ts">
import { useI18n } from "../i18n";
import type { ConnectionPhase } from "../types";

defineProps<{
  page: string;
  recording: boolean;
  blePhase: ConnectionPhase | null;
}>();

const emit = defineEmits<{ navigate: [page: string] }>();
const { t } = useI18n();

const items = [
  { id: "connection", icon: "◉" },
  { id: "buttons", icon: "⊞" },
  { id: "stats", icon: "▤" },
  { id: "history", icon: "◷" },
  { id: "diagnostics", icon: "⚙" },
  { id: "simulator", icon: "▶" },
  { id: "about", icon: "ⓘ" },
] as const;
</script>

<template>
  <nav class="sidebar">
    <div class="brand">
      <span class="logo-dot">声</span>
      <span>{{ t("app.name") }}</span>
    </div>
    <button
      v-for="item in items"
      :key="item.id"
      class="nav-item"
      :class="{ active: page === item.id }"
      @click="emit('navigate', item.id)"
    >
      <span class="icon">{{ item.icon }}</span>
      <span>{{ t(`nav.${item.id}` as never) }}</span>
    </button>
    <div class="voice-indicator">
      <span class="pulse-dot" :class="{ recording }"></span>
      <span v-if="recording">语音中…</span>
      <span v-else-if="blePhase === 'ready'">遥控器已连接</span>
      <span v-else>{{ t("app.tagline") }}</span>
    </div>
  </nav>
</template>
