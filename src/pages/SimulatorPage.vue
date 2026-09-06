<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";
import { useI18n } from "../i18n";
import type { UiEvent } from "../types";
import RemoteCanvas from "../components/RemoteCanvas.vue";

const { t } = useI18n();

interface Receipt {
  id: number;
  button: string;
  gesture: string;
  action: string;
  ok: boolean;
  at: string;
}

const props = defineProps<{ physicalActiveButtons?: Set<string> }>();

const receipts = ref<Receipt[]>([]);
const voiceBusy = ref(false);
const voiceLevel = ref(0);
const recording = ref(false);
const gestureMode = ref<"single" | "double" | "long">("single");
const selected = ref<string | null>(null);
let receiptSeq = 0;

const BUTTON_IDS = [
  "power", "up", "left", "ok", "right", "down", "back",
  "volume_up", "home", "volume_down", "menu", "tv",
] as const;

// 本地活动高亮（叠加物理按键事件）。
const localActive = ref(new Set<string>());

async function press(button: string) {
  const next = new Set(localActive.value);
  next.add(button);
  localActive.value = next;
  try {
    await api.simulateButton(button, gestureMode.value);
  } finally {
    window.setTimeout(() => {
      const clear = new Set(localActive.value);
      clear.delete(button);
      localActive.value = clear;
    }, 280);
  }
}

const mergedActive = computed(() => {
  if (!props.physicalActiveButtons?.size) return localActive.value;
  const merged = new Set(localActive.value);
  for (const id of props.physicalActiveButtons) merged.add(id);
  return merged;
});

async function testVoice() {
  voiceBusy.value = true;
  await api.simulateVoice(2000);
}

onMounted(async () => {
  await listen<UiEvent>("bridge://event", (event) => {
    const payload = event.payload;
    if (payload.type === "ActionReceipt") {
      receipts.value.unshift({
        id: receiptSeq++,
        button: payload.button,
        gesture: payload.gesture,
        action: payload.action,
        ok: payload.ok,
        at: new Date().toLocaleTimeString(),
      });
      receipts.value = receipts.value.slice(0, 12);
    } else if (payload.type === "VoiceState") {
      recording.value = payload.recording;
      voiceLevel.value = payload.level;
      if (!payload.recording) voiceBusy.value = false;
    }
  });
});
</script>

<template>
  <div class="page">
    <h1>{{ t("sim.title") }}</h1>
    <p class="page-sub">{{ t("sim.hint") }}</p>

    <div style="display: grid; grid-template-columns: 280px 1fr; gap: 14px; align-items: start">
      <section class="card" style="margin-bottom: 0">
        <RemoteCanvas
          :selected="selected"
          :active-buttons="mergedActive"
          :voice-active="recording"
          @select="press"
        />
        <div class="row" style="justify-content: center; margin-top: 12px">
          <button
            v-for="mode in ['single', 'double', 'long'] as const"
            :key="mode"
            class="btn"
            :class="{ primary: gestureMode === mode }"
            @click="gestureMode = mode"
          >
            {{ t(`buttons.slot.${mode}` as never) }}
          </button>
        </div>
      </section>

      <div>
        <section class="card">
          <h3>{{ t("sim.voice") }}</h3>
          <div class="row">
            <button class="btn primary" :disabled="voiceBusy" @click="testVoice">
              {{ t("sim.voice") }}
            </button>
            <div class="level-bar">
              <div class="fill" :style="{ width: `${Math.min(100, voiceLevel * 140)}%` }"></div>
            </div>
          </div>
          <p class="hint" style="margin-top: 10px">{{ t("sim.voice_hint") }}</p>
        </section>

        <section class="card">
          <h3>{{ t("sim.recent") }}</h3>
          <div v-if="receipts.length" class="history-list">
            <div v-for="receipt in receipts" :key="receipt.id" class="history-item">
              <div class="row">
                <span class="status-dot" :class="receipt.ok ? 'ok' : 'fail'"></span>
                <strong>{{ receipt.button }}</strong>
                <span class="badge">{{ receipt.gesture }}</span>
                <span>→ {{ receipt.action }}</span>
              </div>
              <span class="meta">{{ receipt.at }}</span>
            </div>
          </div>
          <div v-else class="empty">{{ t("common.empty") }}</div>
        </section>
      </div>
    </div>
  </div>
</template>
