<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "../i18n";
import { actionLabel } from "../actionLabel";
import type { ButtonAction } from "../types";

const props = defineProps<{
  buttonId: string;
  slot: "single" | "double" | "long";
  current: ButtonAction;
}>();

const emit = defineEmits<{
  pick: [action: ButtonAction];
  close: [];
}>();

const { t } = useI18n();
const tab = ref<"basic" | "system" | "custom" | "apps">("basic");

const basicPresets: Array<{ id: string; action: ButtonAction }> = [
  { id: "hands_free", action: { kind: "trigger_hands_free" } },
  { id: "escape", action: { kind: "shortcut", vk: 0x1b, modifiers: 0, label: "Esc" } },
  { id: "enter", action: { kind: "shortcut", vk: 0x0d, modifiers: 0, label: "Enter" } },
  { id: "ctrl_enter", action: { kind: "shortcut", vk: 0x0d, modifiers: 2, label: "Ctrl+Enter" } },
  { id: "shift_enter", action: { kind: "shortcut", vk: 0x0d, modifiers: 4, label: "Shift+Enter" } },
  { id: "copy", action: { kind: "shortcut", vk: 0x43, modifiers: 2, label: "Ctrl+C" } },
  { id: "paste", action: { kind: "shortcut", vk: 0x56, modifiers: 2, label: "Ctrl+V" } },
  { id: "cut", action: { kind: "shortcut", vk: 0x58, modifiers: 2, label: "Ctrl+X" } },
  { id: "select_all", action: { kind: "shortcut", vk: 0x41, modifiers: 2, label: "Ctrl+A" } },
  { id: "undo", action: { kind: "shortcut", vk: 0x5a, modifiers: 2, label: "Ctrl+Z" } },
  { id: "redo", action: { kind: "shortcut", vk: 0x59, modifiers: 2, label: "Ctrl+Y" } },
  { id: "find", action: { kind: "shortcut", vk: 0x46, modifiers: 2, label: "Ctrl+F" } },
  { id: "save", action: { kind: "shortcut", vk: 0x53, modifiers: 2, label: "Ctrl+S" } },
  { id: "new", action: { kind: "shortcut", vk: 0x4e, modifiers: 2, label: "Ctrl+N" } },
  { id: "delete", action: { kind: "shortcut", vk: 0x2e, modifiers: 0, label: "Delete" } },
  { id: "backspace", action: { kind: "shortcut", vk: 0x08, modifiers: 0, label: "Backspace" } },
  { id: "up", action: { kind: "shortcut", vk: 0x26, modifiers: 0, label: "↑" } },
  { id: "down", action: { kind: "shortcut", vk: 0x28, modifiers: 0, label: "↓" } },
  { id: "left", action: { kind: "shortcut", vk: 0x25, modifiers: 0, label: "←" } },
  { id: "right", action: { kind: "shortcut", vk: 0x27, modifiers: 0, label: "→" } },
  { id: "page_up", action: { kind: "shortcut", vk: 0x21, modifiers: 0, label: "PgUp" } },
  { id: "page_down", action: { kind: "shortcut", vk: 0x22, modifiers: 0, label: "PgDn" } },
  { id: "browser_back", action: { kind: "shortcut", vk: 0xa6, modifiers: 0, label: "↩" } },
  { id: "browser_forward", action: { kind: "shortcut", vk: 0xa7, modifiers: 0, label: "↪" } },
];

const systemPresets: Array<{ id: string; action: ButtonAction }> = [
  { id: "volume_up", action: { kind: "volume_up" } },
  { id: "volume_down", action: { kind: "volume_down" } },
  { id: "volume_mute", action: { kind: "volume_mute" } },
  { id: "play_pause", action: { kind: "media_key", code: "play_pause" } },
  { id: "next", action: { kind: "media_key", code: "next" } },
  { id: "previous", action: { kind: "media_key", code: "previous" } },
  { id: "show_desktop", action: { kind: "show_desktop" } },
  { id: "task_view", action: { kind: "task_view" } },
  { id: "app_switcher", action: { kind: "app_switcher" } },
  { id: "screenshot_region", action: { kind: "screenshot", region: true } },
  { id: "screenshot_full", action: { kind: "screenshot", region: false } },
  { id: "click_confirm", action: { kind: "click_confirm" } },
  { id: "open_settings", action: { kind: "open_settings" } },
];

// 录制快捷键（WebView 内键盘事件 → Windows VK 对齐）。
const recording = ref(false);
const customVk = ref(0);
const customModifiers = ref(0);
const customLabel = ref("");

const MOD_CONTROL = 2;
const MOD_SHIFT = 4;
const MOD_ALT = 1;
const MOD_WIN = 8;

function specialLabel(vk: number): string | null {
  const map: Record<number, string> = {
    0x08: "Backspace", 0x09: "Tab", 0x0d: "Enter", 0x1b: "Esc", 0x20: "Space",
    0x21: "PgUp", 0x22: "PgDn", 0x23: "End", 0x24: "Home",
    0x25: "←", 0x26: "↑", 0x27: "→", 0x28: "↓", 0x2d: "Insert", 0x2e: "Delete",
  };
  return map[vk] ?? null;
}

function vkToLabel(vk: number): string {
  const special = specialLabel(vk);
  if (special) return special;
  if (vk >= 0x70 && vk <= 0x7b) return `F${vk - 0x6f}`;
  if (vk >= 0x30 && vk <= 0x39) return String.fromCharCode(vk);
  if (vk >= 0x41 && vk <= 0x5a) return String.fromCharCode(vk);
  return `0x${vk.toString(16).toUpperCase()}`;
}

function onRecordKey(event: KeyboardEvent) {
  if (!recording.value) return;
  event.preventDefault();
  event.stopPropagation();
  // 忽略纯修饰键按下。
  if (["Control", "Shift", "Alt", "Meta"].includes(event.key)) return;
  const vk = event.which || event.keyCode;
  if (!vk) return;
  let modifiers = 0;
  if (event.ctrlKey) modifiers |= MOD_CONTROL;
  if (event.shiftKey) modifiers |= MOD_SHIFT;
  if (event.altKey) modifiers |= MOD_ALT;
  if (event.metaKey) modifiers |= MOD_WIN;
  customVk.value = vk;
  customModifiers.value = modifiers;
  const parts = [
    modifiers & MOD_CONTROL ? "Ctrl" : "",
    modifiers & MOD_SHIFT ? "Shift" : "",
    modifiers & MOD_ALT ? "Alt" : "",
    modifiers & MOD_WIN ? "Win" : "",
  ].filter(Boolean);
  customLabel.value = [...parts, vkToLabel(vk)].join("+");
  recording.value = false;
}

onMounted(() => window.addEventListener("keydown", onRecordKey, true));
onBeforeUnmount(() => window.removeEventListener("keydown", onRecordKey, true));

function confirmCustomShortcut() {
  if (!customVk.value) return;
  emit("pick", {
    kind: "shortcut",
    vk: customVk.value,
    modifiers: customModifiers.value,
    label: customLabel.value || vkToLabel(customVk.value),
  });
}

const appPath = ref("");
const appName = ref("");
const url = ref("");

function confirmApp() {
  const target = appPath.value.trim();
  if (!target || !appName.value.trim()) return;
  emit("pick", { kind: "open_app", target, label: appName.value.trim() });
}

function confirmUrl() {
  const value = url.value.trim();
  if (!value) return;
  emit("pick", { kind: "open_url", url: value.startsWith("https://") ? value : `https://${value}` });
}

const isCurrent = computed(
  () => (action: ButtonAction) => JSON.stringify(action) === JSON.stringify(props.current),
);
</script>

<template>
  <div class="action-picker-overlay" @click.self="emit('close')">
    <div class="action-picker">
      <header>
        <div class="row between">
          <h3>{{ t("buttons.action.change") }}</h3>
          <button class="btn subtle" @click="emit('close')">✕</button>
        </div>
      </header>
      <div class="picker-tabs">
        <button
          v-for="id in ['basic', 'system', 'custom', 'apps'] as const"
          :key="id"
          class="picker-tab"
          :class="{ active: tab === id }"
          @click="tab = id"
        >
          {{ t(`buttons.action.tab.${id}` as never) }}
        </button>
      </div>

      <div class="picker-list" v-if="tab === 'basic'">
        <button
          class="picker-item"
          :class="{ current: isCurrent(preset.action) }"
          v-for="preset in basicPresets"
          :key="preset.id"
          @click="emit('pick', preset.action)"
        >
          <span>{{ actionLabel(preset.action, t) }}</span>
          <span v-if="isCurrent(preset.action)">✓</span>
        </button>
      </div>

      <div class="picker-list" v-else-if="tab === 'system'">
        <button
          class="picker-item"
          :class="{ current: isCurrent(preset.action) }"
          v-for="preset in systemPresets"
          :key="preset.id"
          @click="emit('pick', preset.action)"
        >
          <span>{{ actionLabel(preset.action, t) }}</span>
          <span v-if="isCurrent(preset.action)">✓</span>
        </button>
      </div>

      <div class="picker-list" v-else-if="tab === 'custom'" style="padding: 16px">
        <div class="setting-row">
          <div class="label">{{ t("buttons.action.recorder") }}</div>
          <button class="btn" @click="recording = !recording">
            {{ recording ? t("buttons.action.recording.stop") : t("buttons.action.recorder") }}
          </button>
        </div>
        <p v-if="recording" class="hint">{{ t("buttons.action.recording") }}</p>
        <div v-if="customLabel" class="row" style="margin: 10px 0">
          <span class="badge">{{ customLabel }}</span>
          <span class="spacer"></span>
          <button class="btn primary" @click="confirmCustomShortcut">
            {{ t("common.confirm") }}
          </button>
        </div>
        <div class="setting-row">
          <div class="label">{{ t("buttons.action.disabled") }}</div>
          <button class="btn danger" @click="emit('pick', { kind: 'disabled' })">
            {{ t("buttons.action.clear") }}
          </button>
        </div>
      </div>

      <div class="picker-list" v-else style="padding: 16px; gap: 10px">
        <div>
          <div class="label" style="font-size: 13px; margin-bottom: 4px">{{ t("buttons.action.apps.open_app") }}</div>
          <div class="row">
            <input v-model="appPath" type="text" placeholder="C:\...\app.exe · cursor://" style="flex: 1" />
            <input v-model="appName" type="text" :placeholder="t('buttons.action.apps.name_placeholder')" style="width: 110px" />
            <button class="btn" @click="confirmApp">{{ t("common.confirm") }}</button>
          </div>
        </div>
        <div>
          <div class="label" style="font-size: 13px; margin-bottom: 4px">{{ t("buttons.action.apps.open_url") }}</div>
          <div class="row">
            <input v-model="url" type="text" placeholder="example.com" style="flex: 1" />
            <button class="btn" @click="confirmUrl">{{ t("common.confirm") }}</button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
