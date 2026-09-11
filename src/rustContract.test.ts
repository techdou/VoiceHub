import { describe, expect, it } from "vitest"
import { absentButtonsForModel } from "./canvasLayout"

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

/** PascalCase 枚举名 → 前端 button id（缺席集成员恰为全小写单词）。 */
function toButtonId(variant: string): string {
  return variant.toLowerCase()
}

/** 从 remote_model.rs 的 absent_buttons 提取 Rc003 缺席键（PascalCase 列表）。 */
function rustRc003Absent(): string[] {
  const code = readSource(rustSources, "remote_model.rs")
  const block = code.match(
    /pub fn absent_buttons\([\s\S]*?RemoteModel::Rc003 => \{([\s\S]*?)\}/,
  )
  expect(block, "RemoteModel::absent_buttons Rc003 arm not found").toBeTruthy()
  return [...block![1].matchAll(/RemoteButton::(\w+)/g)].map((m) => m[1])
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
  it("absent-button set matches RemoteModel::absent_buttons (RC003)", () => {
    const rust = rustRc003Absent().map(toButtonId).sort()
    const frontend = [...absentButtonsForModel("Mi Remote Control 2 Pro RC003")].sort()
    expect(frontend).toEqual(rust)
    // 权威侧声明的缺失键非空（防止解析失配后两边同时为空还绿）。
    expect(rust.length).toBeGreaterThanOrEqual(3)
  })

  it("frontend absent set stays empty for unmeasured models", () => {
    expect(absentButtonsForModel(null).size).toBe(0)
    expect(absentButtonsForModel("Mi Remote Control 2 RC001").size).toBe(0)
    expect(absentButtonsForModel("whatever").size).toBe(0)
  })

  it("ProviderKind literals match the Rust enum wire names", () => {
    expect(frontendProviderLiterals()).toEqual(rustProviderWireNames())
  })
})
