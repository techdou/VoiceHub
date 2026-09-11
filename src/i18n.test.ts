import { describe, expect, it } from "vitest";
import { dict, formatDuration, resolveLocale, setLocale, useI18n } from "./i18n";
import type { I18nKey } from "./i18n";

describe("i18n dictionary integrity", () => {
  it("zh and en expose the same key set", () => {
    const zhKeys = Object.keys(dict.zh).sort();
    const enKeys = Object.keys(dict.en).sort();
    expect(enKeys).toEqual(zhKeys);
  });

  it("no empty translations", () => {
    for (const [key, value] of Object.entries(dict.en)) {
      expect(value.length, `empty translation: ${key}`).toBeGreaterThan(0);
    }
  });

  it("falls back to the key itself for unknown keys", () => {
    const { t } = useI18n();
    setLocale("en");
    expect(t("__nonexistent__" as I18nKey)).toBe("__nonexistent__");
  });
});

describe("locale resolution", () => {
  it("explicit settings win over system", () => {
    expect(resolveLocale("zh_cn", "en-US")).toBe("zh");
    expect(resolveLocale("english", "zh-CN")).toBe("en");
  });

  it("system locale used when following system", () => {
    expect(resolveLocale("system", "zh-CN")).toBe("zh");
    expect(resolveLocale("system", "en-US")).toBe("en");
  });
});

describe("duration formatting", () => {
  it("formats milliseconds to human strings", () => {
    expect(formatDuration(500)).toBe("500ms");
    expect(formatDuration(1500)).toBe("1.5s");
    expect(formatDuration(90_000)).toBe("1m 30s");
  });
});

describe("template i18n reference integrity", () => {
  // t() 对缺失 key 的兜底是显示 key 原文（界面上直接露出
  // "connection.provider.sayit" 这种串），模板里的 `as never` 又绕过了
  // 编译期检查——这里在测试层补网（源码经 ?raw 读取，与 appApiContract 同路数）。
  const sources = import.meta.glob("/src/**/*.{vue,ts}", {
    query: "?raw",
    import: "default",
    eager: true,
  }) as Record<string, string>;
  const zhKeys = new Set(Object.keys(dict.zh));
  const pages = Object.entries(sources).filter(([path]) => !path.includes(".test."));

  it("every static t(\"...\") reference exists in the dictionary", () => {
    for (const [path, code] of pages) {
      const refs = [...code.matchAll(/t\(\s*["']([^"']+)["']/g)].map((m) => m[1]);
      for (const key of refs) {
        expect(zhKeys.has(key), `${path}: missing key ${key}`).toBe(true);
      }
    }
  });

  it("template-key prefixes match at least one dictionary key", () => {
    for (const [path, code] of pages) {
      const prefixes = [...code.matchAll(/t\(\s*`([^`${}]+)\$\{/g)].map((m) => m[1]);
      for (const prefix of prefixes) {
        const hit = [...zhKeys].some((k) => k.startsWith(prefix));
        expect(hit, `${path}: prefix "${prefix}" has no keys`).toBe(true);
      }
    }
  });

  // 回归：providerOptions 的枚举值曾缺 key（sayit/none 在首次引导里显示成
  // key 原文）。枚举值从源码提取（单一真理），不另维护清单。
  it("provider option literals resolve to dictionary keys", () => {
    const seen = Object.entries(sources).filter(([path, code]) => code.includes("providerOptions"));
    expect(seen.length).toBeGreaterThanOrEqual(2);
    for (const [path, code] of seen) {
      const m = code.match(/providerOptions\s*=\s*\[([^\]]+)\]/);
      expect(m, `${path}: providerOptions literal not found`).toBeTruthy();
      const kinds = [...m![1].matchAll(/"([^"]+)"/g)].map((x) => x[1]);
      expect(kinds.length).toBeGreaterThan(0);
      for (const kind of kinds) {
        expect(zhKeys.has(`connection.provider.${kind}`), `${path}: connection.provider.${kind}`).toBe(true);
      }
    }
  });
});
