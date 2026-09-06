<script setup lang="ts">
import { useI18n } from "../i18n";

/// RC003 真机布局遥控器图：顶部双键（左电源/右语音）、中部圆盘导航、
/// 左列 返回/主页/菜单、右列 音量±/TV。键位百分比与连线锚点一致
/// （见 canvasLayout.ts 的 PLACEMENTS 标定数据）。

const props = withDefaults(
  defineProps<{
    selected?: string | null;
    activeButtons?: Set<string>;
    voiceActive?: boolean;
    clickable?: boolean;
  }>(),
  {
    selected: null,
    activeButtons: () => new Set<string>(),
    voiceActive: false,
    clickable: true,
  },
);

const emit = defineEmits<{ select: [button: string] }>();
const { t } = useI18n();

interface KeySpec {
  id: string;
  label: string;
  /** 中心点（相对遥控器 202×410，即锚点）。 */
  x: number;
  y: number;
  w: number;
  h: number;
  round: boolean;
}

const keys: KeySpec[] = [
  { id: "power", label: "⏻", x: 38.6, y: 9.9, w: 34, h: 34, round: true },
  { id: "up", label: "▲", x: 50.2, y: 17.9, w: 30, h: 30, round: true },
  { id: "left", label: "◀", x: 36.2, y: 24.6, w: 30, h: 30, round: true },
  { id: "ok", label: "OK", x: 50.2, y: 24.6, w: 42, h: 42, round: true },
  { id: "right", label: "▶", x: 63.8, y: 24.6, w: 30, h: 30, round: true },
  { id: "down", label: "▼", x: 50.2, y: 31.7, w: 30, h: 30, round: true },
  { id: "back", label: "←", x: 40.6, y: 38.9, w: 36, h: 24, round: false },
  { id: "home", label: "⌂", x: 40.6, y: 47.9, w: 36, h: 24, round: false },
  { id: "menu", label: "≡", x: 40.6, y: 56.9, w: 36, h: 24, round: false },
  { id: "volume_up", label: "＋", x: 60.4, y: 39.0, w: 30, h: 26, round: false },
  { id: "volume_down", label: "－", x: 60.4, y: 48.0, w: 30, h: 26, round: false },
  { id: "tv", label: "TV", x: 60.4, y: 56.9, w: 30, h: 26, round: false },
];

function style(key: KeySpec) {
  return {
    left: `${key.x}%`,
    top: `${key.y}%`,
    width: `${key.w}px`,
    height: `${key.h}px`,
    marginLeft: `${-key.w / 2}px`,
    marginTop: `${-key.h / 2}px`,
    borderRadius: key.round ? "50%" : "999px",
  };
}

function isActive(id: string) {
  return props.activeButtons.has(id);
}
</script>

<template>
  <div class="rc-body">
    <!-- 圆盘轮廓：中心 (50.2%, 24.8%)，直径约 96px -->
    <div class="rc-dish"></div>
    <button
      v-for="key in keys"
      :key="key.id"
      class="rc-key"
      :class="{
        selected: selected === key.id,
        active: isActive(key.id),
      }"
      :style="style(key)"
      :disabled="!clickable"
      @click="emit('select', key.id)"
    >
      {{ key.label }}
    </button>
    <div class="rc-voice" :class="{ active: voiceActive }">
      <span class="rc-mic">🎙</span>
      <span class="rc-voice-label">{{ t("buttons.voice_key") }}</span>
    </div>
  </div>
</template>

<style scoped>
.rc-body {
  position: relative;
  width: 202px;
  height: 410px;
  border-radius: 26px;
  background: linear-gradient(170deg, #2e3542, #20242d);
  border: 1px solid rgba(255, 255, 255, 0.07);
  box-shadow:
    0 1px 3px rgba(0, 0, 0, 0.3),
    0 8px 24px rgba(0, 0, 0, 0.25);
  flex-shrink: 0;
}

.rc-dish {
  position: absolute;
  left: 50.2%;
  top: 24.8%;
  width: 96px;
  height: 96px;
  margin: -48px 0 0 -48px;
  border-radius: 50%;
  border: 1.5px solid rgba(255, 255, 255, 0.1);
  background: rgba(255, 255, 255, 0.03);
  pointer-events: none;
}

.rc-key {
  position: absolute;
  display: grid;
  place-items: center;
  padding: 0;
  border: 1px solid rgba(255, 255, 255, 0.13);
  background: rgba(255, 255, 255, 0.06);
  color: #d7dbe2;
  font-size: 11px;
  line-height: 1;
  cursor: pointer;
  transition:
    background 0.12s,
    transform 0.06s,
    border-color 0.12s;
}

.rc-key:hover {
  background: rgba(255, 255, 255, 0.14);
}

.rc-key:active {
  transform: scale(0.93);
}

.rc-key:disabled {
  cursor: default;
}

.rc-key.selected {
  background: var(--accent);
  border-color: transparent;
  color: #fff;
  font-weight: 700;
}

.rc-key.active {
  border-color: rgba(232, 163, 61, 0.85);
  background: rgba(232, 163, 61, 0.28);
  color: #f4c37a;
}

.rc-voice {
  position: absolute;
  left: 63%;
  top: 9.9%;
  transform: translate(-50%, -50%);
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 10px;
  border-radius: 999px;
  border: 1px solid rgba(232, 163, 61, 0.5);
  background: rgba(232, 163, 61, 0.16);
  color: var(--accent);
  font-size: 10px;
  white-space: nowrap;
  pointer-events: none;
}

.rc-voice.active {
  background: var(--accent);
  color: #fff;
  animation: rc-pulse 1s infinite;
}

.rc-mic {
  font-size: 11px;
}

@keyframes rc-pulse {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.55;
  }
}
</style>
