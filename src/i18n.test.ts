import { afterEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import {
  getLanguageState, normalizeLanguagePreference, resolveLocale, setLanguagePreference,
  subscribeLanguage, t, tr, updateSystemLanguage,
} from "./i18n";
import { terminalCompletionPreferencesFromSettings } from "./terminal-completion-prefs";

afterEach(() => { vi.unstubAllGlobals(); });

describe("language identifiers and presentation", () => {
  it.each([
    ["ar-SA", "ar"], ["ar_EG.UTF-8", "ar"], ["zh-Hant-TW", "zh"], ["zh_CN", "zh"],
    ["en-US", "en"], ["fr_CA.UTF-8", "fr"], ["ru-RU", "ru"], ["es-MX", "es"],
    ["de-DE", "en"], ["ja-JP", "en"], ["C", "en"], ["", "en"], [null, "en"],
  ])("resolves %s to %s", (input, expected) => { expect(resolveLocale(input)).toBe(expected); });

  it("stores only language identifiers, never localized names", () => {
    const setItem = vi.fn();
    vi.stubGlobal("window", { localStorage: { setItem } });
    for (const locale of ["ar", "zh", "en", "fr", "ru", "es", "system"] as const) {
      setLanguagePreference(locale);
      expect(setItem).toHaveBeenLastCalledWith("portmate.language.v1", locale);
    }
    for (const input of ["العربية", "中文", "English", "fr-FR", null, {}]) {
      expect(normalizeLanguagePreference(input)).toBe("system");
    }
  });

  it("preserves explicit choice across system changes and uses the new system locale when selected", () => {
    setLanguagePreference("fr");
    updateSystemLanguage("ar-SA");
    expect(getLanguageState()).toEqual({ preference: "fr", locale: "fr" });
    setLanguagePreference("system");
    expect(getLanguageState()).toEqual({ preference: "system", locale: "ar" });
    updateSystemLanguage("de-DE");
    expect(getLanguageState().locale).toBe("en");
  });

  it("notifies subscribers only on changes and remains usable when persistence fails", () => {
    vi.stubGlobal("window", { localStorage: { setItem: () => { throw Error("blocked"); } } });
    const listener = vi.fn();
    const unsubscribe = subscribeLanguage(listener);
    setLanguagePreference("ar");
    setLanguagePreference("ar");
    expect(listener).toHaveBeenCalledTimes(1);
    expect(getLanguageState().locale).toBe("ar");
    unsubscribe();
    setLanguagePreference("en");
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("does not recursively translate or interpolate user data", () => {
    const userData = 'Session العربية 中文 {1} <script> /tmp/save';
    expect(t("failed", [userData], "en")).toBe(`${userData} failed`);
    expect(t("__proto__", [], "ar")).toBe("__proto__");
    expect(t("no-such-message", [], "ar")).toBe("no-such-message");
  });

  it("preserves rich React nodes while escaping text", () => {
    setLanguagePreference("en");
    const html = renderToStaticMarkup(createElement("p", null, tr("failed", [createElement("strong", null, "<client>")])));
    expect(html).toBe("<p><strong>&lt;client&gt;</strong> failed</p>");
    expect(html).not.toContain("[object Object]");
  });

  it("keeps numeric and English settings identical under every UI language", () => {
    const settings = { completionTriggerChars: 3, completionListHeight: 10, completionPreviewMode: "top" };
    setLanguagePreference("en");
    const expected = terminalCompletionPreferencesFromSettings(settings);
    for (const locale of ["ar", "zh", "en", "fr", "ru", "es"] as const) {
      setLanguagePreference(locale);
      expect(terminalCompletionPreferencesFromSettings(settings)).toEqual(expected);
    }
    expect(terminalCompletionPreferencesFromSettings({ completionTriggerChars: "3 字符", completionListHeight: "10 行", completionPreviewMode: "列表顶部" }))
      .toMatchObject({ triggerCharacters: 1, listRows: 7, previewMode: "none" });
  });
});
