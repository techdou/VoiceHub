import { describe, expect, it } from "vitest";

// Vite ?raw 导入：把源码当字符串读进来做契约断言（避免引入 @types/node）。
import typesSource from "./types.ts?raw";
import appVueSource from "./App.vue?raw";
import connectionPageSource from "./pages/ConnectionPage.vue?raw";
import simulatorPageSource from "./pages/SimulatorPage.vue?raw";
import statsPageSource from "./pages/StatsPage.vue?raw";
import historyPageSource from "./pages/HistoryPage.vue?raw";

// 前后端事件契约：UiEvent 的 type 判别值必须与 Rust 端
// src-tauri/src/bridge.rs 的 UiEvent 序列化输出（PascalCase）一致。
// Rust 侧对应测试：bridge::tests::ui_event_tags_match_frontend_contract。
const EXPECTED_TAGS = [
  "BleState",
  "VoiceState",
  "Battery",
  "ActionReceipt",
  "ButtonActivity",
  "ShowSettings",
  "AudioEndpointChanged",
];

describe("UiEvent 前后端契约", () => {
  it("types.ts 的判别值与 Rust 序列化输出一致", () => {
    for (const tag of EXPECTED_TAGS) {
      expect(typesSource).toContain(`type: "${tag}"`);
    }
    // 防回退：camelCase 的 tag 是 H2 事故的根因。
    expect(typesSource).not.toContain('"bleState"');
  });

  it("事件消费方（App.vue / 各页面）只使用契约内的 tag", () => {
    const consumers: Record<string, string> = {
      "src/App.vue": appVueSource,
      "src/pages/ConnectionPage.vue": connectionPageSource,
      "src/pages/SimulatorPage.vue": simulatorPageSource,
      "src/pages/StatsPage.vue": statsPageSource,
      "src/pages/HistoryPage.vue": historyPageSource,
    };
    for (const [file, source] of Object.entries(consumers)) {
      const used = [...source.matchAll(/(?:case|payload\.type ===|===)\s*"([A-Za-z]+)"/g)]
        .map((m) => m[1])
        .filter((name) => /^[A-Z][a-z]/.test(name) && name.endsWith("e"));
      for (const tag of used) {
        expect(EXPECTED_TAGS, `${file} 使用了契约外的 tag "${tag}"`).toContain(tag);
      }
    }
  });
});
