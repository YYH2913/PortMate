import { afterEach, describe, expect, it, vi } from "vitest";
import {
  DEFAULT_UI_APPEARANCE,
  UI_APPEARANCE_STORAGE_KEY,
  getUiAppearance,
  normalizeUiAppearance,
  setUiAppearance,
  setUiAppearancePreview,
  subscribeUiAppearance,
} from "../../src/ui-theme";

afterEach(() => {
  setUiAppearancePreview(null);
  setUiAppearance(DEFAULT_UI_APPEARANCE);
  vi.unstubAllGlobals();
});

describe("ui appearance theme, layout and accent store", () => {
  it("defaults to studio-light with docked layout and default accent", () => {
    expect(DEFAULT_UI_APPEARANCE).toEqual({
      theme: "studio-light",
      layout: "docked",
      accent: "default",
    });
    expect(normalizeUiAppearance(null)).toEqual(DEFAULT_UI_APPEARANCE);
    expect(normalizeUiAppearance({ theme: "invalid", layout: "weird", accent: 123 })).toEqual(DEFAULT_UI_APPEARANCE);
  });

  it("persists normalized mix-and-match choices and notifies subscribers only on changes", () => {
    const setItem = vi.fn();
    vi.stubGlobal("window", { localStorage: { setItem } });
    const listener = vi.fn();
    const unsubscribe = subscribeUiAppearance(listener);

    setUiAppearance({ theme: "obsidian-linear", layout: "islands", accent: "emerald" });
    expect(getUiAppearance()).toEqual({ theme: "obsidian-linear", layout: "islands", accent: "emerald" });
    expect(setItem).toHaveBeenLastCalledWith(
      UI_APPEARANCE_STORAGE_KEY,
      JSON.stringify({ theme: "obsidian-linear", layout: "islands", accent: "emerald" }),
    );
    expect(listener).toHaveBeenCalledTimes(1);

    setUiAppearance({ theme: "obsidian-linear", layout: "islands", accent: "emerald" });
    expect(listener).toHaveBeenCalledTimes(1);

    setUiAppearance({ theme: "carbon-islands" });
    expect(getUiAppearance()).toEqual({ theme: "carbon-islands", layout: "islands", accent: "emerald" });
    expect(listener).toHaveBeenCalledTimes(2);

    unsubscribe();
  });

  it("applies live preview to documentElement and reverts cleanly when cleared", () => {
    const dataset: Record<string, string> = {};
    vi.stubGlobal("document", { documentElement: { dataset } });
    setUiAppearance({ theme: "studio-light", layout: "islands", accent: "default" });
    expect(dataset).toEqual({ uiTheme: "studio-light", uiLayout: "islands", uiAccent: "default" });

    setUiAppearancePreview({ theme: "carbon-islands", layout: "docked", accent: "amber" });
    expect(dataset).toEqual({ uiTheme: "carbon-islands", uiLayout: "docked", uiAccent: "amber" });
    expect(getUiAppearance()).toEqual({ theme: "studio-light", layout: "islands", accent: "default" });

    setUiAppearancePreview(null);
    expect(dataset).toEqual({ uiTheme: "studio-light", uiLayout: "islands", uiAccent: "default" });
  });
});
