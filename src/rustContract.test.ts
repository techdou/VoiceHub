import { describe, expect, it } from "vitest"
import type { RemoteButtonId } from "./types"

/// 前后端契约对照：前端展示副本 vs Rust 语义权威，两边源码经 ?raw 读取
/// （与 appApiContract/uiEventContract 同路数）。改任一侧而忘同步另一侧时，
/// 这里会先红——此前两对"双份真理"（absent 集合、ProviderKind wire 名）
/// 只靠注释互相提醒，没有机器保证。

const rustSources = import.meta.glob("/crates/voicehub-core/src/*.rs", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>
const frontendSources = import.meta.glob("/src/*.{ts,vue}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>

function readSource(map: Record<string, string>, suffix: string): string {
  const hit = Object.entries(map).find(([path]) => path.endsWith(suffix))
  expect(hit, `source not found: *${suffix}`).toBeTruthy()
  return hit![1]
}

/** 从 buttons.rs 的 RemoteButton 枚举提取 wire 名（snake_case）。 */
function rustRemoteButtonWires(): string[] {
  const code = readSource(rustSources, "buttons.rs")
  const block = code.match(/pub enum RemoteButton \{([\s\S]*?)\n\}/)
  expect(block, "RemoteButton enum not found").toBeTruthy()
  return [...block![1].matchAll(/^\s*(\w+),?$/gm)].map((m) =>
    m[1].replace(/([a-z])([A-Z])/g, "$1_$2").toLowerCase(),
  ).sort()
}

/** 从 types.ts 提取 RemoteButtonId 联合。 */
function frontendRemoteButtonIds(): string[] {
  const code = readSource(frontendSources, "types.ts")
  const m = code.match(/export type RemoteButtonId[^=]*=([\s\S]*?);/)
  expect(m, "frontend RemoteButtonId union not found").toBeTruthy()
  return [...m![1].matchAll(/"([^"]+)"/g)].map((x) => x[1]).sort()
}

/** 从 provider.rs 枚举声明提取 wire 名（含 #[serde(rename)] 覆盖）。 */
function rustProviderWireNames(): string[] {
  const code = readSource(rustSources, "provider.rs")
  const block = code.match(/pub enum ProviderKind \{([\s\S]*?)\n\}/)
  expect(block, "ProviderKind enum not found").toBeTruthy()
  const wires: string[] = []
  let pendingRename: string | null = null
  for (const rawLine of block![1].split("\n")) {
    const line = rawLine.trim()
    const rename = line.match(/^#\[serde\(rename = "([^"]+)"\)\]$/)
    if (rename) {
      pendingRename = rename[1]
      continue
    }
    const variant = line.match(/^(\w+),?$/)
    if (variant) {
      // snake_case 默认：SayIt → say_it，显式 rename 优先。
      wires.push(pendingRename ?? variant[1].replace(/([a-z])([A-Z])/g, "$1_$2").toLowerCase())
      pendingRename = null
    }
  }
  return wires.sort()
}

/** 从 types.ts 提取 ProviderKind 字面量联合。 */
function frontendProviderLiterals(): string[] {
  const code = readSource(frontendSources, "types.ts")
  const m = code.match(/export type ProviderKind = ([^;]+);/)
  expect(m, "frontend ProviderKind union not found").toBeTruthy()
  return [...m![1].matchAll(/"([^"]+)"/g)].map((x) => x[1]).sort()
}

describe("rust ↔ frontend contract", () => {
  it("RemoteButtonId matches the Rust RemoteButton wire names", () => {
    expect(frontendRemoteButtonIds()).toEqual(rustRemoteButtonWires())
    // 7 键模型锚点（RC003 真机定案）：防止两侧同时漂移回 12 键。
    expect(rustRemoteButtonWires()).toEqual([
      "down", "home", "menu", "ok", "up", "volume_down", "volume_up",
    ])
  })

  it("ProviderKind literals match the Rust enum wire names", () => {
    expect(frontendProviderLiterals()).toEqual(rustProviderWireNames())
  })
})
