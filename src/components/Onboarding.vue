<script setup lang="ts">
import { computed, ref } from "vue";
import { openUrl, openPath } from "@tauri-apps/plugin-opener";
import { api } from "../api";
import { useI18n } from "../i18n";
import CableInstaller from "./CableInstaller.vue";
import type { AppSettings, AudioEndpoint, PairedRemote } from "../types";

const props = defineProps<{ settings: AppSettings }>();
const emit = defineEmits<{
  "update-settings": [settings: AppSettings];
  finish: [];
}>();

const { t } = useI18n();
const step = ref(1);
const remotes = ref<PairedRemote[]>([]);
const endpoints = ref<AudioEndpoint[]>([]);
const selectedRemoteId = ref("");
const busy = ref(false);

async function refreshRemotes() {
  remotes.value = await api.listPairedRemotes();
}

async function loadEndpoints() {
  endpoints.value = await api.listAudioEndpoints();
}

async function pickRemote(id: string, name: string) {
  selectedRemoteId.value = id;
  await api.connectRemote(id, name);
}

async function chooseEndpoint(id: string, name: string) {
  await api.selectAudioEndpoint(id, name);
}

async function chooseProvider(kind: AppSettings["provider"]["kind"]) {
  const nextSettings: AppSettings = {
    ...props.settings,
    provider: { ...props.settings.provider, kind },
  };
  emit("update-settings", nextSettings);
}

async function next() {
  busy.value = true;
  try {
    if (step.value === 1) {
      await refreshRemotes();
    } else if (step.value === 2) {
      await loadEndpoints();
    }
    if (step.value === 4) {
      emit("update-settings", { ...props.settings, onboardingComplete: true });
      emit("finish");
      return;
    }
    step.value++;
  } finally {
    busy.value = false;
  }
}

function back() {
  step.value = Math.max(1, step.value - 1);
}

const headline = computed(() => t(`onboarding.step${step.value}.title` as never));
const body = computed(() => t(`onboarding.step${step.value}.body` as never));
const providerOptions = ["sayit", "custom", "none"] as const;
</script>

<template>
  <div class="onboarding-overlay">
    <div class="onboarding">
      <h2>{{ t("onboarding.welcome") }}</h2>
      <div class="steps">
        <span
          v-for="s in [1, 2, 3, 4]"
          :key="s"
          class="step"
          :class="{ done: s <= step }"
        ></span>
      </div>
      <h3 style="margin-bottom: 6px">{{ headline }}</h3>
      <p>{{ body }}</p>

      <template v-if="step === 1">
        <div class="row">
          <button class="btn" @click="openPath('ms-settings:bluetooth')">
            {{ t("onboarding.open_settings") }}
          </button>
          <button class="btn" @click="refreshRemotes">{{ t("common.refresh") }}</button>
        </div>
        <div v-if="remotes.length" style="margin-top: 12px; display: grid; gap: 6px">
          <button
            v-for="remote in remotes"
            :key="remote.id"
            class="picker-item"
            :class="{ current: selectedRemoteId === remote.id }"
            @click="pickRemote(remote.id, remote.name)"
          >
            <span>{{ remote.name }}</span>
            <span v-if="selectedRemoteId === remote.id">✓</span>
          </button>
        </div>
      </template>

      <template v-else-if="step === 2">
        <div class="row">
          <button class="btn" @click="openUrl('https://vb-audio.com/Cable/')">
            {{ t("onboarding.download_vbcable") }}
          </button>
          <button class="btn" @click="loadEndpoints">{{ t("common.refresh") }}</button>
        </div>
        <div style="margin-top: 12px">
          <CableInstaller @installed="loadEndpoints" />
        </div>
        <div v-if="endpoints.length" style="margin-top: 12px; display: grid; gap: 6px">
          <button
            v-for="endpoint in endpoints"
            :key="endpoint.id"
            class="picker-item"
            :class="{ current: settings.audioEndpointName === endpoint.name }"
            @click="chooseEndpoint(endpoint.id, endpoint.name)"
          >
            <span>{{ endpoint.name }}</span>
            <span v-if="endpoint.isVirtualCableCandidate" class="badge">CABLE</span>
          </button>
        </div>
      </template>

      <template v-else-if="step === 3">
        <div style="display: grid; gap: 6px">
          <button
            v-for="kind in providerOptions"
            :key="kind"
            class="picker-item"
            :class="{ current: settings.provider.kind === kind }"
            @click="chooseProvider(kind)"
          >
            <span>{{ t(`connection.provider.${kind}` as never) }}</span>
          </button>
        </div>
      </template>

      <div class="row between" style="margin-top: 18px">
        <button class="btn subtle" :disabled="step === 1" @click="back">
          {{ t("common.back") }}
        </button>
        <div class="row">
          <button class="btn subtle" @click="emit('finish')">{{ t("common.skip") }}</button>
          <button class="btn primary" :disabled="busy" @click="next">
            {{ step === 4 ? t("common.done") : t("common.next") }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>
