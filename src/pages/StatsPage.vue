<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";
import { formatDuration, useI18n } from "../i18n";
import type { DailyUsage, UiEvent, UsageStatistics } from "../types";

const { t } = useI18n();
const stats = ref<UsageStatistics | null>(null);
const range = ref<"today" | "week" | "recent7" | "all">("today");
let unlisten: (() => void) | undefined;
// mounted 与 unmount 竞态：listen 落定前组件被卸载则清理函数无人调（App.vue 同款）。
let disposed = false;

onMounted(async () => {
  unlisten = await listen<UiEvent>("bridge://event", (event) => {
    if (event.payload.type === "VoiceState" && !event.payload.recording) {
      api.getStatistics().then((next) => (stats.value = next));
    }
  });
  if (disposed) unlisten();
  try {
    stats.value = await api.getStatistics();
  } catch (error) {
    console.error("[soundbridge] load statistics failed:", error);
  }
});

onBeforeUnmount(() => {
  unlisten?.();
  unlisten = undefined;
});

const todayKey = computed(() => {
  const now = new Date();
  return `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`;
});

function emptyUsage(): DailyUsage {
  return { buttonPressCount: 0, buttonCounts: {}, voiceSessionCount: 0, voiceDurationMs: 0, longestVoiceMs: 0 };
}

function aggregate(rangeKey: string): DailyUsage {
  if (!stats.value) return emptyUsage();
  const days = Object.entries(stats.value.days);
  if (rangeKey === "all") {
    return days.reduce(
      (acc, [, day]) => ({
        buttonPressCount: acc.buttonPressCount + day.buttonPressCount,
        buttonCounts: Object.fromEntries(
          [...new Set([...Object.keys(acc.buttonCounts), ...Object.keys(day.buttonCounts)])].map((key) => [
            key,
            (acc.buttonCounts[key] ?? 0) + (day.buttonCounts[key] ?? 0),
          ]),
        ),
        voiceSessionCount: acc.voiceSessionCount + day.voiceSessionCount,
        voiceDurationMs: acc.voiceDurationMs + day.voiceDurationMs,
        longestVoiceMs: Math.max(acc.longestVoiceMs, day.longestVoiceMs),
      }),
      emptyUsage(),
    );
  }
  const now = new Date();
  const keys = new Set<string>();
  if (rangeKey === "today") {
    keys.add(todayKey.value);
  } else if (rangeKey === "week") {
    // 本周按周一起算。
    const monday = new Date(now);
    const offset = (monday.getDay() + 6) % 7;
    for (let i = 0; i <= offset; i++) {
      const day = new Date(monday);
      day.setDate(monday.getDate() - i);
      keys.add(`${day.getFullYear()}-${String(day.getMonth() + 1).padStart(2, "0")}-${String(day.getDate()).padStart(2, "0")}`);
    }
  } else {
    for (let i = 0; i <= 6; i++) {
      const day = new Date(now);
      day.setDate(now.getDate() - i);
      keys.add(`${day.getFullYear()}-${String(day.getMonth() + 1).padStart(2, "0")}-${String(day.getDate()).padStart(2, "0")}`);
    }
  }
  const selected = days.filter(([key]) => keys.has(key));
  return selected.reduce(
    (acc, [, day]) => ({
      buttonPressCount: acc.buttonPressCount + day.buttonPressCount,
      buttonCounts: Object.fromEntries(
        [...new Set([...Object.keys(acc.buttonCounts), ...Object.keys(day.buttonCounts)])].map((key) => [
          key,
          (acc.buttonCounts[key] ?? 0) + (day.buttonCounts[key] ?? 0),
        ]),
      ),
      voiceSessionCount: acc.voiceSessionCount + day.voiceSessionCount,
      voiceDurationMs: acc.voiceDurationMs + day.voiceDurationMs,
      longestVoiceMs: Math.max(acc.longestVoiceMs, day.longestVoiceMs),
    }),
    emptyUsage(),
  );
}

const current = computed(() => aggregate(range.value));

const buttonDistribution = computed(() =>
  Object.entries(current.value.buttonCounts)
    .sort((a, b) => b[1] - a[1])
    .slice(0, 8),
);

const chart = computed(() => {
  const now = new Date();
  const series: Array<{ label: string; ms: number }> = [];
  for (let i = 6; i >= 0; i--) {
    const day = new Date(now);
    day.setDate(now.getDate() - i);
    const key = `${day.getFullYear()}-${String(day.getMonth() + 1).padStart(2, "0")}-${String(day.getDate()).padStart(2, "0")}`;
    series.push({
      label: `${String(day.getMonth() + 1).padStart(2, "0")}-${String(day.getDate()).padStart(2, "0")}`,
      ms: stats.value?.days[key]?.voiceDurationMs ?? 0,
    });
  }
  return series;
});

const chartMax = computed(() => Math.max(1, ...chart.value.map((d) => d.ms)));

// 按键名直接复用 buttons.key_names.* 字典（此前手写中文，英文界面下错乱）。
const buttonNames = computed<Record<string, string>>(() => ({
  power: t("buttons.key_names.power"), up: t("buttons.key_names.up"),
  left: t("buttons.key_names.left"), ok: "OK", right: t("buttons.key_names.right"),
  down: t("buttons.key_names.down"), back: t("buttons.key_names.back"),
  volume_up: t("buttons.key_names.volume_up"), home: t("buttons.key_names.home"),
  volume_down: t("buttons.key_names.volume_down"), menu: t("buttons.key_names.menu"),
  tv: "TV",
}));
</script>

<template>
  <div class="page">
    <h1>{{ t("stats.title") }}</h1>
    <p class="page-sub">{{ t("history.privacy") }}</p>

    <div class="row" style="margin-bottom: 14px">
      <button
        v-for="id in ['today', 'week', 'recent7', 'all'] as const"
        :key="id"
        class="btn"
        :class="{ primary: range === id }"
        @click="range = id"
      >
        {{ t(`stats.${id}` as never) }}
      </button>
    </div>

    <div class="stat-cards">
      <div class="stat-card">
        <div class="value">{{ current.buttonPressCount }}</div>
        <div class="label">{{ t("stats.button_presses") }}</div>
      </div>
      <div class="stat-card">
        <div class="value">{{ current.voiceSessionCount }}</div>
        <div class="label">{{ t("stats.voice_sessions") }}</div>
      </div>
      <div class="stat-card">
        <div class="value">{{ formatDuration(current.voiceDurationMs) }}</div>
        <div class="label">{{ t("stats.voice_duration") }}</div>
      </div>
      <div class="stat-card">
        <div class="value">{{ formatDuration(current.longestVoiceMs) }}</div>
        <div class="label">{{ t("stats.longest") }}</div>
      </div>
    </div>

    <section class="card">
      <h3>{{ t("stats.chart7") }}</h3>
      <div class="bar-chart">
        <div v-for="day in chart" :key="day.label" class="bar-col">
          <div
            class="bar"
            :style="{ height: `${Math.max(2, (day.ms / chartMax) * 100)}%` }"
            :title="formatDuration(day.ms)"
          ></div>
          <span class="bar-label">{{ day.label }}</span>
        </div>
      </div>
    </section>

    <section class="card">
      <h3>{{ t("stats.by_button") }}</h3>
      <div v-if="buttonDistribution.length" class="history-list">
        <div v-for="[button, count] in buttonDistribution" :key="button" class="history-item">
          <strong>{{ buttonNames[button] ?? button }}</strong>
          <span>{{ count }}</span>
        </div>
      </div>
      <div v-else class="empty">{{ t("common.empty") }}</div>
    </section>
  </div>
</template>
