<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { api } from "../api";
import { useI18n } from "../i18n";
import type { AppSettings, ButtonAction, ButtonMapping, CustomShortcut, RemoteButtonId } from "../types";
import ActionPicker from "../components/ActionPicker.vue";
import MappingCanvas from "../components/MappingCanvas.vue";
import PageSkeleton from "../components/PageSkeleton.vue";
import SaveBadge from "../components/SaveBadge.vue";
import { actionLabel as sharedActionLabel } from "../actionLabel";

const props = defineProps<{
  settings: AppSettings | null;
  saveState: "idle" | "saving" | "saved" | "error";
  saveError: string;
  activeButtons?: Set<string>;
  voiceActive?: boolean;
}>();

const emit = defineEmits<{ "update-settings": [settings: AppSettings] }>();
const { t } = useI18n();

// 草稿模式（与连接页一致）：映射改动先落 draft，点「保存」才写盘生效。
// 此前每次编辑都立即写盘——误触即改坏键位，且无"改完再统一保存"的回旋余地。
const draft = ref<Pick<AppSettings, "profiles" | "buttonMappingEnabled"> | null>(null);
watch(
  () => props.settings,
  (next) => {
    if (!next) { draft.value = null; return; }
    // dirty 时跳过整体重置：外部设置更新（连接页等）不含 profiles 变化，
    // 重置会静默丢掉正在编辑的键位草稿。
    if (dirty.value && draft.value) return;
    draft.value = JSON.parse(JSON.stringify({
      profiles: next.profiles,
      buttonMappingEnabled: next.buttonMappingEnabled,
    }));
  },
  { immediate: true },
);

const dirty = computed(() => {
  if (!draft.value || !props.settings) return false;
  const current = {
    profiles: props.settings.profiles,
    buttonMappingEnabled: props.settings.buttonMappingEnabled,
  };
  return JSON.stringify(draft.value) !== JSON.stringify(current);
});

const saving = ref(false);
function save() {
  if (!draft.value || !props.settings || saving.value || !dirty.value) return;
  saving.value = true;
  try {
    emit("update-settings", { ...props.settings, ...draft.value });
  } finally {
    saving.value = false;
  }
}

const selectedButton = ref<RemoteButtonId | null>(null);
const editingSlot = ref<"single" | "double" | "long">("single");
const showPicker = ref(false);

// 按住说话（push-to-talk）：边沿直达的第四通道——按下沿发 ptt-down、释放沿发
// ptt-up（事件直连引擎），让这个遥控器键变成"按住说话"的麦克风触发键。
// 绑定后三槽手势被后端互斥忽略。开关式绑定，无需组合键。
function toggleHoldKey() {
  if (!selectedButton.value) return;
  const button = selectedButton.value;
  const next = !selectedHoldEnabled.value;
  mutateProfiles((profiles) => {
    const profile = profiles.profiles.find((p) => p.id === profiles.selectedProfileId);
    if (!profile) return;
    const binding = profile.mapping.bindings[button] ??= {
      single: { kind: "disabled" }, double: { kind: "disabled" }, long: { kind: "disabled" }, pushToTalk: false,
    };
    binding.pushToTalk = next;
  });
}

const selectedHoldEnabled = computed(() => {
  const button = selectedButton.value;
  if (!button || !activeProfile.value) return false;
  return activeProfile.value.mapping.bindings[button]?.pushToTalk ?? false;
});
const newProfileName = ref("");
const foregroundProcess = ref<string | null>(null);

const SECONDARY_BUTTONS = new Set(["home", "menu", "ok", "tv"]);

const activeProfileId = computed(() => draft.value?.profiles.selectedProfileId ?? "");

const activeProfile = computed(
  () => draft.value?.profiles.profiles.find((p) => p.id === activeProfileId.value) ?? null,
);

function bindingFor(button: RemoteButtonId) {
  return activeProfile.value?.mapping.bindings[button] ?? { single: { kind: "disabled" }, double: { kind: "disabled" }, long: { kind: "disabled" } };
}

function actionLabel(action: ButtonAction): string {
  return sharedActionLabel(action, t);
}
/** 所有编辑只改草稿；保存按钮统一落盘。 */
function mutateProfiles(mutator: (profiles: AppSettings["profiles"]) => void) {
  if (!draft.value) return;
  const profiles = JSON.parse(JSON.stringify(draft.value.profiles)) as AppSettings["profiles"];
  mutator(profiles);
  draft.value = { ...draft.value, profiles };
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
    if (!bindings[button]) bindings[button] = { single: { kind: "disabled" }, double: { kind: "disabled" }, long: { kind: "disabled" }, pushToTalk: false };
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
  if (!draft.value || draft.value.profiles.profiles.length <= 1) return;
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
  if (!draft.value) return;
  draft.value = { ...draft.value, buttonMappingEnabled: enabled };
}

// 进程绑定走 Rust 命令（后端做进程名归一化并直接落盘）——有未保存草稿时禁用，
// 防止后端真值与草稿分叉。resetProfile 同理（出厂映射只存在于 Rust 侧）。
async function bindCurrentApp() {
  if (!activeProfileId.value || dirty.value) return;
  const process = await api.getForegroundProcess();
  if (!process) return;
  foregroundProcess.value = process;
  await api.bindProcessToProfile(process, activeProfileId.value);
  // 绑定在 Rust 侧直接改了设置，重新拉取同步到前端。
  emit("update-settings", await api.getSettings());
}

async function unbindApp(process: string) {
  if (dirty.value) return;
  await api.unbindProcess(process);
  emit("update-settings", await api.getSettings());
}

const smartBindings = computed(() =>
  Object.entries(draft.value?.profiles.rules.processBindings ?? {}),
);

// 恢复默认映射（Rust 侧出厂映射）。dirty 时禁用：它直接改后端真值，会与草稿分叉。
async function resetProfile() {
  if (!activeProfileId.value || dirty.value) return;
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
  <PageSkeleton v-if="!(settings && draft)" :title="t('buttons.title')" />
  <div class="page" v-else>
    <header class="page-head">
      <div>
        <h1>{{ t("buttons.title") }}</h1>
        <p class="page-sub">{{ t("buttons.canvas.hint") }}</p>
      </div>
      <span v-if="dirty" class="dirty-mark">{{ t("connection.unsaved_changes") }}</span>
      <button class="btn" :class="{ primary: dirty }" :disabled="!dirty || saving" @click="save">
        {{ saving ? t("common.saving") : t("common.save") }}
      </button>
      <SaveBadge :state="saveState" :error="saveError" />
      <div class="head-switch">
        <span class="head-switch-label">{{ t("buttons.mapping.toggle") }}</span>
        <button
          class="switch"
          :class="{ on: draft.buttonMappingEnabled }"
          :aria-label="t('buttons.mapping.toggle')"
          @click="toggleMapping(!draft.buttonMappingEnabled)"
        ></button>
      </div>
    </header>

    <div class="toolbar card">
      <div class="row">
        <button
          v-for="profile in draft.profiles.profiles"
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
          <button class="btn subtle" :disabled="dirty" :title="dirty ? t('buttons.dirty_note') : undefined" @click="resetProfile">{{ t("buttons.profile.reset") }}</button>
          <button class="btn subtle" @click="exportProfile">{{ t("buttons.profile.export") }}</button>
          <button class="btn subtle" @click="openImport">{{ t("buttons.profile.import") }}</button>
          <button
            v-if="draft.profiles.profiles.length > 1"
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
          :class="{ on: draft.profiles.smartEnabled }"
          :aria-label="t('buttons.smart')"
          @click="toggleSmart(!draft.profiles.smartEnabled)"
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
      <!-- 按住说话：边沿直达通道，绕过单击/双击/长按判定（与三槽互斥，见 mapping.rs）。 -->
      <div v-if="selectedButton" class="setting-row" style="margin-top: 14px; border-top: 1px solid var(--border); padding-top: 14px">
        <div>
          <div class="label">{{ t("buttons.hold.title") }}</div>
          <div class="desc">
            {{ selectedHoldEnabled ? t("buttons.hold.bound") : t("buttons.hold.empty") }}
          </div>
        </div>
        <button
          class="switch"
          :class="{ on: selectedHoldEnabled }"
          :aria-label="t('buttons.hold.title')"
          @click="toggleHoldKey"
        ></button>
      </div>
    </section>

    <section v-if="draft.profiles.smartEnabled" class="card">
      <h3>{{ t("buttons.smart.bindings") }}</h3>
      <div class="row" style="margin: 8px 0 10px">
        <button class="btn" :disabled="dirty" :title="dirty ? t('buttons.dirty_note') : undefined" @click="bindCurrentApp">
          {{ t("buttons.smart.bind_current") }}
        </button>
        <span v-if="foregroundProcess" class="badge">{{ foregroundProcess }} → {{ activeProfile?.name }}</span>
      </div>
      <div v-if="smartBindings.length" class="history-list">
        <div v-for="[process, profileId] in smartBindings" :key="process" class="history-item">
          <div>
            <strong>{{ process }}</strong>
            <span style="color: var(--text-secondary)">
              → {{ draft.profiles.profiles.find((p) => p.id === profileId)?.name ?? "?" }}
            </span>
          </div>
          <button class="btn subtle" :disabled="dirty" @click="unbindApp(process)">✕</button>
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
</template>

<style scoped>
.dirty-mark {
  font-size: 12px;
  color: var(--warn, #c08a2d);
  white-space: nowrap;
}

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
