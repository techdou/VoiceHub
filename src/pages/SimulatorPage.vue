<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";
import { useI18n } from "../i18n";
import { absentButtonsForModel } from "../canvasLayout";
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

const props = defineProps<{ physicalActiveButtons?: Set<string>; providerKind?: string; remoteModel?: string | null }>();

// 此型号机身上不存在的键：淡显提示（语义权威见 canvasLayout.ts）。
const absentButtons = computed(() => absentButtonsForModel(props.remoteModel));

const receipts = ref<Receipt[]>([]);
const voiceBusy = ref(false);
const voiceError = ref("");
const fileInput = ref<HTMLInputElement | null>(null);

async function testAudioFile(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  if (!file) return;
  voiceError.value = "";
  // 解码前的粗筛：完整解码 1 小时的音频会先吃掉数百 MB 内存再被拒绝。
  // 16kHz/16bit/mono 的 300s PCM ≈ 9.6MB，常见压缩比 >4:1，60MB 上限足够宽松。
  if (file.size > 60 * 1024 * 1024) {
    voiceError.value = t("sim.audio_too_long");
    input.value = "";
    return;
  }
  voiceBusy.value = true;
  let context: AudioContext | null = null;
  try {
    context = new AudioContext();
    const decoded = await context.decodeAudioData(await file.arrayBuffer());
    if (decoded.duration > 300) throw new Error(t("sim.audio_too_long"));
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
  finally { if (context) await context.close().catch(() => {}); input.value = ""; }
}
const voiceLevel = ref(0);
const recording = ref(false);
const gestureMode = ref<"single" | "double" | "long">("single");
const selected = ref<string | null>(null);
let receiptSeq = 0;
let unlisten: (() => void) | undefined;
// mounted 与 unmount 竞态：listen 的 Promise 落定前组件就被卸载的话，
// 清理函数没人调，监听器泄漏（App.vue 同款 disposed 模式）。
let disposed = false;

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

// 内置真实语音素材（public/test-speech-15s.pcm，16kHz mono s16le）：
// 走与文件上传相同的 PCM 直通管线，绕开 AudioContext 解码，附带原文可核对识别准确性。
let realSpeechCached: string | null = null;
async function testRealSpeech() {
  voiceBusy.value = true;
  voiceError.value = "";
  try {
    if (!realSpeechCached) {
      const bytes = new Uint8Array(await (await fetch("/test-speech-15s.pcm")).arrayBuffer());
      let binary = "";
      for (let i = 0; i < bytes.length; i += 8192) binary += String.fromCharCode(...bytes.subarray(i, i + 8192));
      realSpeechCached = btoa(binary);
    }
    await api.simulateVoice(undefined, realSpeechCached);
  } catch (error) {
    voiceError.value = String(error);
    voiceBusy.value = false;
  }
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
  if (disposed) unlisten();
});

onBeforeUnmount(() => {
  disposed = true;
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
          :absent-buttons="absentButtons"
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
          <!-- 默认（sayit 直连）路径 PCM 不经音频端点发声，"听到声音"的提示会误导用户判障。 -->
          <p class="hint" style="margin-top: 10px">
            {{ props.providerKind === 'sayit' ? t("sim.voice_hint_direct") : t("sim.voice_hint") }}
          </p>
          <input ref="fileInput" type="file" accept="audio/*" hidden @change="testAudioFile" />
          <div class="row" style="margin-top: 10px; flex-wrap: wrap">
            <button class="btn" :disabled="voiceBusy" @click="fileInput?.click()">{{ t("sim.audio_file") }}</button>
            <button class="btn primary" :disabled="voiceBusy" @click="testRealSpeech">
              {{ t("sim.real_speech") }}
            </button>
          </div>
          <p class="hint" style="margin-top: 8px">{{ t("sim.real_speech_hint") }}</p>
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
