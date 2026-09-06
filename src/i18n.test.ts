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
