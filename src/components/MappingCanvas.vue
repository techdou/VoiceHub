<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "../i18n";
import { actionLabel } from "../actionLabel";
import RemoteCanvas from "./RemoteCanvas.vue";
import {
  allCards,
  allLinks,
  CANVAS,
  canvasScale,
  remoteOrigin,
  type CanvasButtonId,
} from "../canvasLayout";
import type { ButtonAction, ButtonBinding, RemoteButtonId } from "../types";

/// 连线画布：左卡片列 | 遥控器 | 右卡片列，SVG 贝塞尔连线带箭头；
/// 选中键连线加粗变色，物理按下时锚点橙点 + 卡片橙描边。
/// 设计尺寸固定 740×650，容器更窄时整体等比缩放。

const props = defineProps<{
  bindings: Record<string, ButtonBinding>;
  selected: string | null;
  activeButtons: Set<string>;
  voiceActive: boolean;
  /** 不支持双击/长按的按键（二级槽禁用置灰）。 */
  secondaryButtons: Set<string>;
}>();

const emit = defineEmits<{
  selectButton: [button: string];
  editSlot: [button: string, slot: "single" | "double" | "long"];
}>();

const { t } = useI18n();

const SLOTS = ["single", "double", "long"] as const;
type Slot = (typeof SLOTS)[number];

const buttonNames: Record<string, string> = {
  power: "电源", up: "上", left: "左", ok: "OK", right: "右", down: "下",
  back: "返回", volume_up: "音量+", home: "主页", volume_down: "音量−",
  menu: "菜单", tv: "TV",
};

const cards = allCards();
const links = allLinks();
const origin = remoteOrigin();

function bindingOf(button: string): ButtonBinding {
  return (
    props.bindings[button] ?? {
      single: { kind: "disabled" } as ButtonAction,
      double: { kind: "disabled" } as ButtonAction,
      long: { kind: "disabled" } as ButtonAction,
    }
  );
}

function slotAction(button: string, slot: Slot): ButtonAction {
  return bindingOf(button)[slot];
}

function slotEnabled(button: string, slot: Slot): boolean {
  return slot === "single" || props.secondaryButtons.has(button);
}

function linkState(button: string) {
  return {
    selected: props.selected === button,
    active: props.activeButtons.has(button),
  };
}

const voiceBox = computed(() => cards.find((card) => card.button === "voice")!);

// 容器测宽 → 等比缩放。
const containerEl = ref<HTMLElement | null>(null);
const scale = ref(1);
let observer: ResizeObserver | null = null;

function updateScale() {
  if (containerEl.value) {
    scale.value = canvasScale(containerEl.value.clientWidth);
  }
}

onMounted(() => {
  updateScale();
  observer = new ResizeObserver(updateScale);
  if (containerEl.value) observer.observe(containerEl.value);
});

onBeforeUnmount(() => observer?.disconnect());
</script>

<template>
  <div ref="containerEl" class="mc-outer">
    <div
      class="mc-canvas"
      :style="{ transform: `scale(${scale})`, height: `${CANVAS.height * scale}px` }"
    >
      <div class="mc-inner" :style="{ width: `${CANVAS.width}px`, height: `${CANVAS.height}px` }">
        <!-- 连线层 -->
        <svg
          class="mc-links"
          :width="CANVAS.width"
          :height="CANVAS.height"
          viewBox="0 0 740 650"
        >
          <template v-for="link in links" :key="link.button">
            <path
              :d="link.path"
              fill="none"
              :stroke="
                linkState(link.button).selected
                  ? 'var(--accent)'
                  : linkState(link.button).active
                    ? 'rgba(232, 163, 61, 0.8)'
                    : 'rgba(128, 128, 128, 0.4)'
              "
              :stroke-width="linkState(link.button).selected ? 1.8 : 1"
              stroke-linecap="round"
            />
            <polygon
              :points="link.arrow.map((p) => `${p.x},${p.y}`).join(' ')"
              :fill="
                linkState(link.button).selected
                  ? 'var(--accent)'
                  : linkState(link.button).active
                    ? 'rgba(232, 163, 61, 0.8)'
                    : 'rgba(128, 128, 128, 0.5)'
              "
            />
            <circle
              v-if="linkState(link.button).active"
              :cx="link.start.x"
              :cy="link.start.y"
              r="4.5"
              fill="#e8a33d"
            />
          </template>
        </svg>

        <!-- 遥控器 -->
        <div class="mc-remote" :style="{ left: `${origin.x}px`, top: `${origin.y}px` }">
          <RemoteCanvas
            :selected="selected"
            :active-buttons="activeButtons"
            :voice-active="voiceActive"
            @select="emit('selectButton', $event)"
          />
        </div>

        <!-- 映射卡片 -->
        <div
          v-for="card in cards.filter((c) => c.button !== 'voice')"
          :key="card.button"
          class="mc-card"
          :class="{
            selected: selected === card.button,
            active: activeButtons.has(card.button),
          }"
          :style="{ left: `${card.x}px`, top: `${card.y}px`, width: `${card.width}px` }"
          @click="emit('selectButton', card.button)"
        >
          <div class="mc-card-head">
            <strong>{{ buttonNames[card.button] ?? card.button }}</strong>
          </div>
          <div class="mc-slots">
            <button
              v-for="slot in SLOTS"
              :key="slot"
              class="mc-slot"
              :class="{ disabled: !slotEnabled(card.button, slot) }"
              :disabled="!slotEnabled(card.button, slot)"
              :title="`${buttonNames[card.button] ?? card.button} · ${t(`buttons.slot.${slot}` as never)}`"
              @click.stop="emit('editSlot', card.button, slot)"
            >
              <span class="mc-slot-trigger">{{ t(`buttons.slot.${slot}` as never) }}</span>
              <span class="mc-slot-action">{{ actionLabel(slotAction(card.button, slot), t) }}</span>
            </button>
          </div>
        </div>

        <!-- 语音键卡片（固定，不可编辑） -->
        <div
          class="mc-card voice"
          :class="{ active: voiceActive }"
          :style="{ left: `${voiceBox.x}px`, top: `${voiceBox.y}px`, width: `${voiceBox.width}px` }"
        >
          <div class="mc-card-head">
            <strong>🎙 {{ t("buttons.voice_key") }}</strong>
            <span class="mc-fixed" :class="{ on: voiceActive }">{{ t("buttons.voice.fixed") }}</span>
          </div>
          <div class="mc-voice-detail">{{ t("buttons.voice.detail") }}</div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.mc-outer {
  width: 100%;
  overflow: hidden;
}

.mc-canvas {
  transform-origin: top center;
  margin: 0 auto;
  width: 740px;
}

.mc-inner {
  position: relative;
}

.mc-links {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
}

.mc-remote {
  position: absolute;
  z-index: 1;
}

.mc-card {
  position: absolute;
  z-index: 1;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--panel);
  padding: 7px 9px;
  cursor: pointer;
  transition: border-color 0.12s, background 0.12s;
}

.mc-card:hover {
  border-color: var(--border-strong);
}

.mc-card.selected {
  border-color: var(--accent);
  background: var(--accent-soft);
}

.mc-card.active {
  border-color: rgba(232, 163, 61, 0.8);
  background: rgba(232, 163, 61, 0.1);
}

.mc-card-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 12px;
  margin-bottom: 5px;
}

.mc-slots {
  display: grid;
  grid-template-columns: 1fr 1fr 1fr;
  gap: 4px;
}

.mc-slot {
  border: none;
  background: rgba(128, 128, 128, 0.1);
  border-radius: 6px;
  padding: 4px 4px 3px;
  cursor: pointer;
  display: grid;
  gap: 1px;
  text-align: left;
  min-width: 0;
  color: var(--text);
  font: inherit;
}

.mc-slot:hover {
  background: var(--accent-soft);
}

.mc-slot.disabled {
  opacity: 0.35;
  cursor: default;
}

.mc-slot.disabled:hover {
  background: rgba(128, 128, 128, 0.1);
}

.mc-slot-trigger {
  font-size: 10px;
  color: var(--text-secondary);
}

.mc-slot-action {
  font-size: 11px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.mc-fixed {
  font-size: 10px;
  padding: 1px 7px;
  border-radius: 999px;
  background: rgba(128, 128, 128, 0.15);
  color: var(--text-secondary);
}

.mc-fixed.on {
  background: rgba(232, 163, 61, 0.18);
  color: var(--accent);
}

.mc-voice-detail {
  font-size: 11px;
  color: var(--text-secondary);
  line-height: 1.45;
}
</style>
