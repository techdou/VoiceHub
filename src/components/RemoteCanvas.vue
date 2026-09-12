<script setup lang="ts">
import { useI18n } from "../i18n";
import { CANVAS, VOICE_ANCHOR } from "../canvasLayout";
import remotePhoto from "../assets/rc003-remote.webp";

/// RC003 实物照片遥控器：底图为真机照片（已按 alpha 边界裁剪，比例 1:4.065），
/// 键位热区为覆盖在照片上的透明按钮，中心点与 canvasLayout.ts 的
/// PLACEMENTS 连线锚点共用同一套照片标定坐标，像素级对齐。

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
  /** 中心点（相对遥控器框 0–100 百分比，按实物照片标定）。 */
  x: number;
  y: number;
  /** 热区尺寸（px，相对 152 宽遥控器框）。 */
  w: number;
  h: number;
  round: boolean;
}

const keys: KeySpec[] = [
  { id: "power", x: 24.2, y: 6.4, w: 44, h: 44, round: true },
  { id: "up", x: 50.2, y: 13.6, w: 30, h: 30, round: true },
  { id: "left", x: 19.8, y: 21.1, w: 30, h: 30, round: true },
  { id: "ok", x: 50.2, y: 21.1, w: 56, h: 56, round: true },
  { id: "right", x: 80.6, y: 21.1, w: 30, h: 30, round: true },
  { id: "down", x: 50.2, y: 28.6, w: 30, h: 30, round: true },
  { id: "back", x: 29.4, y: 36.0, w: 44, h: 44, round: true },
  { id: "home", x: 29.4, y: 45.3, w: 44, h: 44, round: true },
  { id: "menu", x: 29.5, y: 54.6, w: 44, h: 44, round: true },
  { id: "volume_up", x: 70.3, y: 36.3, w: 44, h: 50, round: false },
  { id: "volume_down", x: 70.3, y: 45.0, w: 44, h: 50, round: false },
  { id: "tv", x: 70.3, y: 54.7, w: 44, h: 44, round: true },
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
  <div
    class="rc-body"
    :style="{ width: `${CANVAS.remote.width}px`, height: `${CANVAS.remote.height}px` }"
  >
    <img class="rc-photo" :src="remotePhoto" :alt="t('buttons.canvas.remote_alt')" draggable="false" />
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
      :aria-label="t(`buttons.key_names.${key.id}` as never)"
      @click="emit('select', key.id)"
    ></button>
    <div
      class="rc-voice"
      :class="{ active: voiceActive }"
      :style="{ left: `${VOICE_ANCHOR.x * 100}%`, top: `${VOICE_ANCHOR.y * 100}%` }"
      :title="t('buttons.voice_key')"
    >
      <span class="rc-mic">🎙</span>
    </div>
  </div>
</template>

<style scoped>
.rc-body {
  position: relative;
  flex-shrink: 0;
  filter: drop-shadow(0 10px 24px rgba(0, 0, 0, 0.16));
}

.rc-photo {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: fill;
  user-select: none;
  pointer-events: none;
}

.rc-key {
  position: absolute;
  padding: 0;
  border: 1.5px solid transparent;
  background: transparent;
  cursor: pointer;
  transition:
    background 0.12s,
    border-color 0.12s,
    transform 0.06s;
}

.rc-key:hover {
  background: rgba(255, 255, 255, 0.16);
}

.rc-key:active {
  transform: scale(0.93);
}

.rc-key:disabled {
  cursor: default;
}

.rc-key.selected {
  background: color-mix(in srgb, var(--accent) 32%, transparent);
  border-color: var(--accent);
}

.rc-key.active {
  border-color: var(--accent);
  background: var(--accent-soft);
}

.rc-voice {
  position: absolute;
  transform: translate(-50%, -50%);
  display: grid;
  place-items: center;
  width: 26px;
  height: 26px;
  border-radius: 50%;
  border: 1px solid var(--accent-soft);
  background: rgba(30, 30, 30, 0.55);
  color: var(--text-secondary);
  font-size: 12px;
  pointer-events: none;
}

.rc-voice.active {
  background: var(--accent);
  border-color: transparent;
  color: #fff;
  animation: rc-pulse 1s infinite;
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
