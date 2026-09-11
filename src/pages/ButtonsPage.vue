<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "../i18n";
import type { AppSettings, ButtonAction, RemoteButtonId } from "../types";
import ActionPicker from "../components/ActionPicker.vue";
import MappingCanvas from "../components/MappingCanvas.vue";
import PageSkeleton from "../components/PageSkeleton.vue";
import SaveBadge from "../components/SaveBadge.vue";
import { actionLabel as sharedActionLabel } from "../actionLabel";
import { absentButtonsForModel } from "../canvasLayout";

const props = defineProps<{
  settings: AppSettings | null;
  saveState: "idle" | "saving" | "saved" | "error";
  saveError: string;
  activeButtons?: Set<string>;
  voiceActive?: boolean;
  /** BLE 2A24 型号串（RC003 等）；null = 未连接。 */
  remoteModel?: string | null;
}>();

const emit = defineEmits<{ "update-settings": [settings: AppSettings] }>();
const { t } = useI18n();

// 即时保存：所有编辑直接写穿到后端（App.vue 乐观更新 + 失败回滚 + SaveBadge
// 反馈）。此前的草稿 + 保存按钮模式让"配好却没生效"成为常态——用户不知道
// 还需点保存；改为改完即生效。
function commit(mutator: (settings: AppSettings) => void) {
  if (!props.settings) return;
  const next = JSON.parse(JSON.stringify(props.settings)) as AppSettings;
  mutator(next);
  emit("update-settings", next);
}

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

const selectedButton = ref<RemoteButtonId | null>(null);
const editingSlot = ref<"single" | "double" | "long">("single");
const showPicker = ref(false);

// 支持双击/长按槽的键（与 Rust supports_secondary 保持一致）。
const SECONDARY_BUTTONS = new Set(["home", "menu", "ok", "tv", "volume_up", "volume_down"]);
// 此型号机身上不存在的键：卡片置灰 + 槽禁用（语义见 canvasLayout.ts 同名函数）。
const ABSENT_BUTTONS = absentButtonsForModel(props.remoteModel);

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
  commit((settings) => {
    const profile = settings.profiles.profiles.find((p) => p.id === activeProfileId.value);
    if (!profile) return;
    const bindings = profile.mapping.bindings;
    if (!bindings[button]) bindings[button] = { single: { kind: "disabled" }, double: { kind: "disabled" }, long: { kind: "disabled" }, pushToTalk: false };
    bindings[button][editingSlot.value] = action;
  });
  showPicker.value = false;
}

// 按住说话（push-to-talk）：边沿直达的第四通道——按下沿发 ptt-down、释放沿发
// ptt-up（事件直连引擎），让这个遥控器键变成"按住说话"的麦克风触发键。
// 开关式绑定，无需组合键；与三槽手势后端互斥忽略。
function toggleHoldKey() {
  if (!selectedButton.value) return;
  const button = selectedButton.value;
  const next = !selectedHoldEnabled.value;
  commit((settings) => {
    const profile = settings.profiles.profiles.find((p) => p.id === settings.profiles.selectedProfileId);
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

function toggleMapping(enabled: boolean) {
  commit((settings) => {
    settings.buttonMappingEnabled = enabled;
  });
}
</script>

<template>
  <PageSkeleton v-if="!settings" :title="t('buttons.title')" />
  <div class="page" v-else>
    <header class="page-head">
      <div>
        <h1>{{ t("buttons.title") }}</h1>
        <p class="page-sub">{{ t("buttons.canvas.hint") }}</p>
      </div>
      <SaveBadge :state="saveState" :error="saveError" />
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

    <section v-if="activeProfile" class="card">
      <MappingCanvas
        :bindings="activeProfile.mapping.bindings"
        :selected="selectedButton"
        :active-buttons="activeButtons ?? new Set()"
        :voice-active="voiceActive ?? false"
        :secondary-buttons="SECONDARY_BUTTONS"
        :absent-buttons="ABSENT_BUTTONS"
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

    <ActionPicker
      v-if="showPicker && selectedButton"
      :button-id="selectedButton"
      :slot="editingSlot"
      :current="bindingFor(selectedButton)[editingSlot]"
      @pick="applyAction"
      @close="showPicker = false"
    />
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
</style>
