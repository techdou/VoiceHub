/**
 * 跨界契约测试：前端实际调用的 invoke 命令 / listen 事件必须在后端有对应实现。
 *
 * 三个契约面：
 * 1. speech 命令表：vendor/sayit/native/src/handler.rs 的 generate_handler 宏
 * 2. hardware 命令表：src-tauri/src/lib.rs 的 HARDWARE_COMMANDS 白名单
 * 3. 后端事件：两个 native 树源码中出现的事件名字符串
 *
 * 这层测试防的是：native 侧改名/漏注册时前端只在运行时才炸（appApi.d.ts 是手写
 * 子集声明，没有编译期约束）。与 uiEventContract.test.ts 同为防漂移锁定。
 * 源码经 Vite ?raw 导入读取（与 uiEventContract 同路数，不引入 @types/node）。
 */
import { describe, expect, test } from "vitest";

// eager glob：构建期把源文件内容作为字符串内联，路径键以 "/" 开头。
// import.meta.glob 的参数必须是字面量（不能包函数传变量）。
type RawSources = Record<string, string>;

const speechHandlerSource = (
  import.meta.glob("/vendor/sayit/native/src/handler.rs", { query: "?raw", import: "default", eager: true }) as RawSources
)["/vendor/sayit/native/src/handler.rs"];
const libRsSource = (
  import.meta.glob("/src-tauri/src/lib.rs", { query: "?raw", import: "default", eager: true }) as RawSources
)["/src-tauri/src/lib.rs"];
const frontendSources: RawSources = {
  ...(import.meta.glob("/vendor/sayit/frontend/src/**/*.{ts,tsx}", { query: "?raw", import: "default", eager: true }) as RawSources),
  ...(import.meta.glob("/src/**/*.{ts,vue}", { query: "?raw", import: "default", eager: true }) as RawSources),
};
const backendSources = Object.values({
  ...(import.meta.glob("/vendor/sayit/native/src/**/*.rs", { query: "?raw", import: "default", eager: true }) as RawSources),
  ...(import.meta.glob("/src-tauri/src/**/*.rs", { query: "?raw", import: "default", eager: true }) as RawSources),
}).join("\n");

/** speech 侧注册命令（handler.rs 宏块内 `path::to::name,` 的最后一段）。 */
function speechCommands(): Set<string> {
  const start = speechHandlerSource.indexOf("generate_handler![");
  const block = speechHandlerSource.slice(start, speechHandlerSource.indexOf("];", start));
  const set = new Set<string>();
  for (const m of block.matchAll(/(\w+)\s*,/g)) set.add(m[1]);
  return set;
}

/** hardware 侧白名单（lib.rs 的 HARDWARE_COMMANDS const）。 */
function hardwareCommands(): Set<string> {
  // 锚定 const 声明本身：注释里也出现过 HARDWARE_COMMANDS 字样，裸 indexOf 会命中注释。
  const start = libRsSource.indexOf("const HARDWARE_COMMANDS");
  const block = libRsSource.slice(start, libRsSource.indexOf("];", start));
  return new Set([...block.matchAll(/"([\w]+)"/g)].map((m) => m[1]));
}

function collect(pattern: RegExp): Map<string, string> {
  const map = new Map<string, string>();
  for (const [file, text] of Object.entries(frontendSources)) {
    for (const m of text.matchAll(pattern)) {
      if (!map.has(m[1])) map.set(m[1], file);
    }
  }
  return map;
}

/** 前端源码里字面量 invoke 的命令名 → 出现文件。 */
const frontendInvokes = () => collect(/\binvoke(?:<[^>]*>)?\(\s*['"]([\w-]+)['"]/g);
/** 前端字面量 listen 的事件名 → 出现文件。 */
const frontendListens = () => collect(/\blisten(?:<[^>]*>)?\(\s*['"]([\w:/-]+)['"]/g);
/** 前端自发事件（自发自听，native 无需存在）。 */
const frontendEmits = () =>
  new Set([...collect(/\bemit\(\s*['"]([\w:/-]+)['"]/g).keys()]);

/** 主应用显式拦截的更新命令（合法存在但被 reject）。 */
const INTERCEPTED_COMMANDS = new Set([
  "download_update",
  "install_downloaded_update",
  "verify_update_package",
]);

describe("frontend ↔ backend ipc contract", () => {
  test("every frontend invoke is registered in handler.rs or HARDWARE_COMMANDS", () => {
    const known = new Set([...speechCommands(), ...hardwareCommands(), ...INTERCEPTED_COMMANDS]);
    expect(known.size).toBeGreaterThan(80); // 解析失败的护栏：两张表不可能这么小
    for (const [command, file] of frontendInvokes()) {
      expect(
        known.has(command),
        `command "${command}" (${file}) is invoked by the frontend but not registered anywhere`,
      ).toBe(true);
    }
  });

  test("every frontend listen target exists in backend sources or frontend emits", () => {
    const selfEmitted = frontendEmits();
    for (const [event, file] of frontendListens()) {
      const inBackend = backendSources.includes(`"${event}"`);
      const inFrontend = selfEmitted.has(event);
      expect(
        inBackend || inFrontend,
        `event "${event}" (${file}) is listened to but never emitted by any backend or frontend emitter`,
      ).toBe(true);
    }
  });
});
