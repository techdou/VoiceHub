<script setup lang="ts">
import { useI18n } from "../i18n";

defineProps<{ version: string }>();
const emit = defineEmits<{ open: [url: string] }>();
const { t } = useI18n();

const references = [
  { name: "vibe-flow（言灵）", url: "https://github.com/techdou/vibe-flow" },
  { name: "remote-mic-app（SayAll）", url: "https://github.com/techdou/remote-mic-app" },
  { name: "remote-mic-app-windows", url: "https://github.com/GetSayAll/remote-mic-app-windows" },
  { name: "whisper.cpp / GGML", url: "https://github.com/ggerganov/whisper.cpp" },
  { name: "Silero VAD", url: "https://github.com/snakers4/silero-vad" },
  { name: "VB-CABLE（虚拟声卡）", url: "https://vb-audio.com/Cable/" },
];
</script>

<template>
  <div class="page" style="max-width: 620px">
    <h1>{{ t("about.title") }}</h1>
    <p class="page-sub">{{ t("app.tagline") }}</p>

    <section class="card row between">
      <div>
        <strong style="font-size: 15px">{{ t("app.name") }} VoiceHub</strong>
        <div class="hint" style="margin: 2px 0 0">
          {{ t("about.version") }} {{ version }} · GPL-3.0
        </div>
      </div>
      <img src="/app-icon.png" width="44" height="44" alt="VoiceHub" style="border-radius: 11px" />
    </section>

    <section class="card">
      <h3>{{ t("about.refs") }}</h3>
      <div class="history-list">
        <button v-for="ref in references" :key="ref.url" class="picker-item" @click="emit('open', ref.url)">
          <span>{{ ref.name }}</span>
          <span style="color: var(--text-secondary)">↗</span>
        </button>
      </div>
      <p class="hint" style="margin-top: 12px">{{ t("about.license") }}</p>
    </section>

    <section class="card">
      <p class="hint" style="margin: 0">{{ t("about.shortcuts") }}</p>
    </section>
  </div>
</template>
