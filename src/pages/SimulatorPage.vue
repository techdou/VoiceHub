<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
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
const voiceError = ref("");
const fileInput = ref<HTMLInputElement | null>(null);

async function testAudioFile(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  if (!file) return;
  voiceError.value = "";
  voiceBusy.value = true;
  const context = new AudioContext();
  try {
    const decoded = await context.decodeAudioData(await file.arrayBuffer());
    if (decoded.duration > 300) throw new Error("测试音频不能超过五分钟");
    const offline = new OfflineAudioContext(1, Math.ceil(decoded.duration * 16000), 16000);
    const source = offline.createBufferSource(); source.buffer = decoded; source.connect(offline.destination); source.start();
    const rendered = await offline.startRendering();
    const samples = rendered.getChannelData(0);
    const pcm = new Int16Array(samples.length);
    for (let i = 0; i < samples.length; i++) pcm[i] = Math.max(-32768, Math.min(32767, Math.round(samples[i] * 32768)));
    const bytes = new Uint8Array(pcm.buffer);
    let binary = "";
    for (let i = 0; i < bytes.length; i += 8192) binary += String.fromCharCode(...bytes.subarray(i, i + 8192));
    await api.simulateVoice(undefined, btoa(binary));
  } catch (error) { voiceError.value = String(error); voiceBusy.value = false; }
  finally { await context.close(); input.value = ""; }
}
const voiceLevel = ref(0);
const recording = ref(false);
const gestureMode = ref<"single" | "double" | "long">("single");
const selected = ref<string | null>(null);
let receiptSeq = 0;
let unlisten: (() => void) | undefined;

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
  voiceError.value = "";
  try { await api.simulateVoice(2000); }
  catch (error) { voiceError.value = String(error); voiceBusy.value = false; }
}

onMounted(async () => {
  unlisten = await listen<UiEvent>("bridge://event", (event) => {
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

onBeforeUnmount(() => {
  unlisten?.();
  unlisten = undefined;
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
          <input ref="fileInput" type="file" accept="audio/*" hidden @change="testAudioFile" />
          <button class="btn" :disabled="voiceBusy" @click="fileInput?.click()">选择音频测试转写</button>
          <p v-if="voiceError" role="alert" class="hint" style="color: var(--fail)">{{ voiceError }}</p>
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
