import { onBeforeUnmount, onMounted, ref, type Ref } from "vue";

export interface RecordedChord {
  vk: number;
  modifiers: number;
}

export function chordLabel(vk: number, modifiers: number): string {
  if (!vk) return "";
  const special: Record<number, string> = {
    0x08: "Backspace", 0x09: "Tab", 0x0d: "Enter", 0x1b: "Esc", 0x20: "Space",
    0x21: "PgUp", 0x22: "PgDn", 0x23: "End", 0x24: "Home",
    0x25: "←", 0x26: "↑", 0x27: "→", 0x28: "↓", 0x2d: "Insert", 0x2e: "Delete",
    0xa0: "Shift", 0xa1: "RShift", 0xa2: "Ctrl", 0xa3: "RCtrl",
    0xa4: "Alt", 0xa5: "RAlt", 0x5b: "Win",
  };
  const base =
    special[vk] ??
    (vk >= 0x70 && vk <= 0x7b
      ? `F${vk - 0x6f}`
      : (vk >= 0x30 && vk <= 0x39) || (vk >= 0x41 && vk <= 0x5a)
        ? String.fromCharCode(vk)
        : `0x${vk.toString(16).toUpperCase()}`);
  const parts = [
    modifiers & 2 ? "Ctrl" : "",
    modifiers & 4 ? "Shift" : "",
    modifiers & 1 ? "Alt" : "",
    modifiers & 8 ? "Win" : "",
  ].filter(Boolean);
  return [...parts, base].join("+");
}

/**
 * 组合键录制器（window 捕获阶段 keydown → Windows VK + MOD 位）。
 * MOD 约定与后端一致：Alt=1 Ctrl=2 Shift=4 Win=8。
 * 提取自 ConnectionPage/ActionPicker 两处重复的录制逻辑。
 */
export function useKeyRecorder(onRecorded: (chord: RecordedChord) => void) {
  const recording = ref(false);
  const handler = (event: KeyboardEvent) => {
    if (!recording.value) return;
    event.preventDefault();
    event.stopPropagation();
    // 纯修饰键按下不算完成（等主键）。
    if (["Control", "Shift", "Alt", "Meta"].includes(event.key)) return;
    const vk = event.which || event.keyCode;
    if (!vk) return;
    let modifiers = 0;
    if (event.ctrlKey) modifiers |= 2;
    if (event.shiftKey) modifiers |= 4;
    if (event.altKey) modifiers |= 1;
    if (event.metaKey) modifiers |= 8;
    recording.value = false;
    onRecorded({ vk, modifiers });
  };
  onMounted(() => window.addEventListener("keydown", handler, true));
  onBeforeUnmount(() => window.removeEventListener("keydown", handler, true));
  return { recording, chordLabel };
}
