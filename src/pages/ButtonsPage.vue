<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "../i18n";
import type { AppSettings, ButtonAction, RemoteButtonId } from "../types";
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

// 即时保存：所有编辑直接写穿到后端（App.vue 乐观更新 + 失败回滚 + SaveBadge
// 反馈）。此前的草稿 + 保存按钮模式让"配好却没生效"成为常态——用户不知道
// 还需点保存；改为改完即生效。
function commit(mutator: (settings: AppSettings) => void) {
  if (!props.settings) return;
  const next = JSON.parse(JSON.stringify(props.settings)) as AppSettings;
  mutator(next);
  emit("update-settings", next);
}

function bindingFor(button: RemoteButtonId) {
  return props.settings?.mapping.bindings[button] ?? { single: { kind: "disabled" }, double: { kind: "disabled" }, long: { kind: "disabled" } };
}

function actionLabel(action: ButtonAction): string {
  return sharedActionLabel(action, t);
}

const selectedButton = ref<RemoteButtonId | null>(null);
const editingSlot = ref<"single" | "double" | "long">("single");
const showPicker = ref(false);

// 支持双击/长按槽的键（与 Rust supports_secondary 保持一致）。
const SECONDARY_BUTTONS = new Set(["home", "menu", "ok", "tv", "volume_up", "volume_down"]);

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
  if (!button || !props.settings) return;
  commit((settings) => {
    const bindings = settings.mapping.bindings;
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
    const binding = settings.mapping.bindings[button] ??= {
      single: { kind: "disabled" }, double: { kind: "disabled" }, long: { kind: "disabled" }, pushToTalk: false,
    };
    binding.pushToTalk = next;
  });
}

const selectedHoldEnabled = computed(() => {
  const button = selectedButton.value;
  if (!button || !props.settings) return false;
  return props.settings.mapping.bindings[button]?.pushToTalk ?? false;
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

    <section v-if="settings" class="card">
      <MappingCanvas
        :bindings="settings.mapping.bindings"
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
