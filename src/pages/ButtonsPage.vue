<script setup lang="ts">
import { computed, ref } from "vue";
import { api } from "../api";
import { useI18n } from "../i18n";
import type { AppSettings, ButtonAction, ButtonMapping, RemoteButtonId } from "../types";
import ActionPicker from "../components/ActionPicker.vue";
import MappingCanvas from "../components/MappingCanvas.vue";
import { actionLabel as sharedActionLabel } from "../actionLabel";

const props = defineProps<{
  settings: AppSettings | null;
  saveTick: number;
  activeButtons?: Set<string>;
  voiceActive?: boolean;
}>();

const emit = defineEmits<{ "update-settings": [settings: AppSettings] }>();
const { t } = useI18n();

const selectedButton = ref<RemoteButtonId | null>(null);
const editingSlot = ref<"single" | "double" | "long">("single");
const showPicker = ref(false);
const newProfileName = ref("");
const foregroundProcess = ref<string | null>(null);

const SECONDARY_BUTTONS = new Set(["home", "menu", "ok", "tv"]);

const activeProfileId = computed(() => props.settings?.profiles.selectedProfileId ?? "");

const activeProfile = computed(
  () => props.settings?.profiles.profiles.find((p) => p.id === activeProfileId.value) ?? null,
);

function bindingFor(button: RemoteButtonId) {
  return activeProfile.value?.mapping.bindings[button] ?? { single: { kind: "disabled" }, double: { kind: "disabled" }, long: { kind: "disabled" } };
}

function actionLabel(action: ButtonAction): string {
  return sharedActionLabel(action, t);
}
function mutateProfiles(mutator: (profiles: AppSettings["profiles"]) => void) {
  if (!props.settings) return;
  const profiles = JSON.parse(JSON.stringify(props.settings.profiles)) as AppSettings["profiles"];
  mutator(profiles);
  emit("update-settings", { ...props.settings, profiles });
}

function selectButton(button: string) {
  selectedButton.value = button as RemoteButtonId;
}

function editSlot(button: string, slot: "single" | "double" | "long") {
  selectedButton.value = button as RemoteButtonId;
  editingSlot.value = slot;
  showPicker.value = true;
}

function applyAction(action: ButtonAction) {
  const button = selectedButton.value;
  if (!button || !activeProfileId.value) return;
  mutateProfiles((profiles) => {
    const profile = profiles.profiles.find((p) => p.id === activeProfileId.value);
    if (!profile) return;
    const bindings = profile.mapping.bindings;
    if (!bindings[button]) bindings[button] = { single: { kind: "disabled" }, double: { kind: "disabled" }, long: { kind: "disabled" } };
    bindings[button][editingSlot.value] = action;
  });
  showPicker.value = false;
}

function selectProfile(id: string) {
  mutateProfiles((profiles) => {
    profiles.selectedProfileId = id;
  });
}

function addProfile() {
  const name = newProfileName.value.trim();
  if (!name) return;
  mutateProfiles((profiles) => {
    const id = `profile-${Date.now().toString(36)}`;
    profiles.profiles.push({
      id, name, icon: "",
      mapping: { bindings: {} },
    });
    profiles.selectedProfileId = id;
  });
  newProfileName.value = "";
}

function removeProfile(id: string) {
  if (!props.settings || props.settings.profiles.profiles.length <= 1) return;
  mutateProfiles((profiles) => {
    profiles.profiles = profiles.profiles.filter((p) => p.id !== id);
    profiles.rules.processBindings = Object.fromEntries(
      Object.entries(profiles.rules.processBindings).filter(([, v]) => v !== id),
    );
    if (!profiles.profiles.find((p) => p.id === profiles.selectedProfileId)) {
      profiles.selectedProfileId = profiles.profiles[0]?.id ?? "";
    }
    if (profiles.rules.fallbackProfileId === id) profiles.rules.fallbackProfileId = "";
  });
  if (selectedButton.value) selectButton(selectedButton.value);
}

function toggleSmart(enabled: boolean) {
  mutateProfiles((profiles) => {
    profiles.smartEnabled = enabled;
  });
}

function toggleMapping(enabled: boolean) {
  if (!props.settings) return;
  emit("update-settings", { ...props.settings, buttonMappingEnabled: enabled });
}

async function bindCurrentApp() {
  if (!activeProfileId.value) return;
  const process = await api.getForegroundProcess();
  if (!process) return;
  foregroundProcess.value = process;
  await api.bindProcessToProfile(process, activeProfileId.value);
  // 绑定在 Rust 侧直接改了设置，重新拉取同步到前端。
  const settings = await api.getSettings();
  emit("update-settings", settings);
}

async function unbindApp(process: string) {
  await api.unbindProcess(process);
  const settings = await api.getSettings();
  emit("update-settings", settings);
}

async function probeForeground() {
  foregroundProcess.value = await api.getForegroundProcess();
}

const smartBindings = computed(() =>
  Object.entries(props.settings?.profiles.rules.processBindings ?? {}),
);

// 恢复默认映射（Rust 侧出厂映射）。
async function resetProfile() {
  if (!activeProfileId.value) return;
  await api.resetProfileToDefault(activeProfileId.value);
  emit("update-settings", await api.getSettings());
}

// 导入导出（剪贴板 JSON）。
const importOpen = ref(false);
const importText = ref("");
const importError = ref("");

async function exportProfile() {
  if (!activeProfile.value) return;
  const payload = JSON.stringify(
    { name: activeProfile.value.name, mapping: activeProfile.value.mapping },
    null,
    2,
  );
  try {
    await navigator.clipboard.writeText(payload);
    alert(t("buttons.profile.exported"));
  } catch {
    // 剪贴板不可用时退回导入框展示，用户手抄。
    importText.value = payload;
    importOpen.value = true;
  }
}

function openImport() {
  importText.value = "";
  importError.value = "";
  importOpen.value = true;
}

async function applyImport() {
  if (!props.settings) return;
  try {
    const parsed = JSON.parse(importText.value) as {
      name?: string;
      mapping: ButtonMapping;
    };
    if (!parsed.mapping || typeof parsed.mapping !== "object") {
      throw new Error("missing mapping");
    }
    mutateProfiles((profiles) => {
      const profile = profiles.profiles.find((p) => p.id === activeProfileId.value);
      if (!profile) return;
      profile.mapping = parsed.mapping;
      if (parsed.name) profile.name = parsed.name;
    });
    importOpen.value = false;
  } catch {
    importError.value = t("buttons.profile.import.invalid");
  }
}
</script>

<template>
  <div class="page" v-if="settings">
    <header class="page-head">
      <div>
        <h1>{{ t("buttons.title") }}</h1>
        <p class="page-sub">{{ t("buttons.canvas.hint") }}</p>
      </div>
      <div class="head-switch">
        <span class="head-switch-label">{{ t("buttons.mapping.toggle") }}</span>
        <button
          class="switch"
          :class="{ on: settings.buttonMappingEnabled }"
          :aria-label="t('buttons.mapping.toggle')"
          @click="toggleMapping(!settings.buttonMappingEnabled)"
        ></button>
      </div>
    </header>

    <div class="toolbar card">
      <div class="row">
        <button
          v-for="profile in settings.profiles.profiles"
          :key="profile.id"
          class="btn"
          :class="{ primary: profile.id === activeProfileId }"
          @click="selectProfile(profile.id)"
        >
          {{ profile.name }}
        </button>
        <input v-model="newProfileName" type="text" :placeholder="t('buttons.profile.new')" style="width: 120px" @keydown.enter="addProfile" />
        <button class="btn" @click="addProfile">{{ t("common.add") }}</button>
        <span class="toolbar-actions">
          <button class="btn subtle" @click="resetProfile">{{ t("buttons.profile.reset") }}</button>
          <button class="btn subtle" @click="exportProfile">{{ t("buttons.profile.export") }}</button>
          <button class="btn subtle" @click="openImport">{{ t("buttons.profile.import") }}</button>
          <button
            v-if="settings.profiles.profiles.length > 1"
            class="btn subtle danger"
            @click="removeProfile(activeProfileId)"
          >
            {{ t("common.delete") }}
          </button>
        </span>
      </div>
      <div class="toolbar-smart">
        <span class="toolbar-smart-label">{{ t("buttons.smart") }}</span>
        <button
          class="switch"
          :class="{ on: settings.profiles.smartEnabled }"
          :aria-label="t('buttons.smart')"
          @click="toggleSmart(!settings.profiles.smartEnabled)"
        ></button>
        <span class="toolbar-smart-hint">{{ t("buttons.smart.hint") }}</span>
      </div>
    </div>

    <section v-if="activeProfile" class="card">
      <MappingCanvas
        :bindings="activeProfile.mapping.bindings"
        :selected="selectedButton"
        :active-buttons="activeButtons ?? new Set()"
        :voice-active="voiceActive ?? false"
        :secondary-buttons="SECONDARY_BUTTONS"
        @select-button="selectButton"
        @edit-slot="editSlot"
      />
    </section>

    <section v-if="settings.profiles.smartEnabled" class="card">
      <h3>{{ t("buttons.smart.bindings") }}</h3>
      <div class="row" style="margin: 8px 0 10px">
        <button class="btn" @click="probeForeground">
          {{ t("buttons.smart.bind_current") }}
        </button>
        <span v-if="foregroundProcess" class="badge">{{ foregroundProcess }} → {{ activeProfile?.name }}</span>
      </div>
      <div v-if="smartBindings.length" class="history-list">
        <div v-for="[process, profileId] in smartBindings" :key="process" class="history-item">
          <div>
            <strong>{{ process }}</strong>
            <span style="color: var(--text-secondary)">
              → {{ settings.profiles.profiles.find((p) => p.id === profileId)?.name ?? "?" }}
            </span>
          </div>
          <button class="btn subtle" @click="unbindApp(process)">✕</button>
        </div>
      </div>
    </section>

    <ActionPicker
      v-if="showPicker && selectedButton"
      :button-id="selectedButton"
      :slot="editingSlot"
      :current="bindingFor(selectedButton)[editingSlot]"
      @pick="applyAction"
      @close="showPicker = false"
    />

    <div v-if="importOpen" class="action-picker-overlay" @click.self="importOpen = false">
      <div class="action-picker import-dialog">
        <header>
          <h3>{{ t("buttons.profile.import") }}</h3>
        </header>
        <div class="import-body">
          <p class="dialog-hint">{{ t("buttons.profile.import.hint") }}</p>
          <textarea
            v-model="importText"
            class="import-box"
            rows="8"
            spellcheck="false"
          ></textarea>
          <p v-if="importError" class="import-error">{{ importError }}</p>
          <div class="row" style="justify-content: flex-end; margin-top: 10px">
            <button class="btn" @click="importOpen = false">{{ t("common.cancel") }}</button>
            <button class="btn primary" @click="applyImport">{{ t("buttons.profile.import") }}</button>
          </div>
        </div>
      </div>
    </div>
  </div>
  <div v-else class="page">
    <p class="empty">{{ t("common.loading") }}</p>
  </div>
</template>

<style scoped>
.page-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
}

.page-head .page-sub {
  margin-bottom: 14px;
}

.head-switch {
  display: flex;
  align-items: center;
  gap: 8px;
  padding-top: 3px;
  flex-shrink: 0;
}

.head-switch-label {
  font-size: 13px;
  color: var(--text-secondary);
  white-space: nowrap;
}

.toolbar {
  padding: 12px 14px;
}

.toolbar-actions {
  display: inline-flex;
  gap: 8px;
  margin-left: auto;
  flex-wrap: nowrap;
}

.toolbar-smart {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 10px;
  padding-top: 10px;
  border-top: 1px solid var(--border);
}

.toolbar-smart-label {
  font-size: 13px;
  white-space: nowrap;
}

.toolbar-smart-hint {
  font-size: 12px;
  color: var(--text-secondary);
}

.import-dialog {
  width: min(480px, 92vw);
}

.import-body {
  padding: 0 18px 16px;
}

.dialog-hint {
  color: var(--text-secondary);
  font-size: 12.5px;
  margin: 0 0 10px;
}

.import-box {
  width: 100%;
  font: 12px/1.5 ui-monospace, Consolas, monospace;
  padding: 8px 10px;
  border-radius: 8px;
  border: 1px solid var(--border-strong);
  background: var(--control);
  color: var(--text);
  resize: vertical;
  user-select: text;
}

.import-error {
  color: var(--fail);
  font-size: 12.5px;
  margin: 6px 0 0;
}
</style>
