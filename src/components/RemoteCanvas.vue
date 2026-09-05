<script setup lang="ts">
import { useI18n } from "../i18n";

defineProps<{
  selected: string | null;
  recording: boolean;
}>();

const emit = defineEmits<{ select: [button: string] }>();
const { t } = useI18n();

const keys3x3: Array<{ id: string; label: string; area: string }> = [
  { id: "back", label: "返回", area: "1 / 1 / 2 / 2" },
  { id: "up", label: "▲", area: "1 / 2 / 2 / 3" },
  { id: "tv", label: "TV", area: "1 / 3 / 2 / 4" },
  { id: "left", label: "◀", area: "2 / 1 / 3 / 2" },
  { id: "ok", label: "OK", area: "2 / 2 / 3 / 3" },
  { id: "right", label: "▶", area: "2 / 3 / 3 / 4" },
  { id: "menu", label: "菜单", area: "3 / 1 / 4 / 2" },
  { id: "down", label: "▼", area: "3 / 2 / 4 / 3" },
  { id: "home", label: "主页", area: "3 / 3 / 4 / 4" },
];
</script>

<template>
  <div class="remote-canvas">
    <div class="remote-body">
      <div class="row" style="justify-content: center">
        <button
          class="remote-key"
          style="width: 44px; height: 44px"
          :class="{ selected: selected === 'power' }"
          @click="emit('select', 'power')"
        >
          ⏻
        </button>
      </div>
      <div class="row" style="justify-content: center; gap: 10px">
        <button
          class="remote-key pill"
          style="width: 56px"
          :class="{ selected: selected === 'volume_up' }"
          @click="emit('select', 'volume_up')"
        >
          ＋
        </button>
        <button
          class="remote-key pill"
          style="width: 56px"
          :class="{ selected: selected === 'volume_down' }"
          @click="emit('select', 'volume_down')"
        >
          －
        </button>
      </div>
      <div
        style="display: grid; grid-template-columns: repeat(3, 1fr); grid-template-rows: repeat(3, 38px); gap: 8px; padding: 0 6px"
      >
        <button
          v-for="key in keys3x3"
          :key="key.id"
          class="remote-key"
          :class="{ selected: selected === key.id }"
          :style="{ gridArea: key.area }"
          @click="emit('select', key.id)"
        >
          {{ key.label }}
        </button>
      </div>
      <button
        class="remote-key voice wide"
        :class="{ active: recording }"
        style="height: 34px"
      >
        {{ t("buttons.voice_key") }}
      </button>
    </div>
  </div>
</template>
