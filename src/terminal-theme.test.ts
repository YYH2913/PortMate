import { describe, expect, it } from "vitest";
import {
  applyTerminalPresentation,
  DEFAULT_TERMINAL_THEME,
  normalizeTerminalTheme,
  terminalTheme,
  TERMINAL_THEME_OPTIONS,
} from "./terminal-theme";

describe("terminal themes", () => {
  it("normalizes unknown and malformed stored theme IDs", () => {
    expect(normalizeTerminalTheme(" graphite ")).toBe("graphite");
    expect(normalizeTerminalTheme("unknown-theme")).toBe(DEFAULT_TERMINAL_THEME);
    expect(normalizeTerminalTheme(null)).toBe(DEFAULT_TERMINAL_THEME);
  });

  it("provides complete, distinct palettes for every selectable theme", () => {
    const backgrounds = new Set<string>();
    for (const option of TERMINAL_THEME_OPTIONS) {
      const theme = terminalTheme(option.value);
      expect(theme.background).toMatch(/^#[0-9a-f]{6}$/i);
      expect(theme.foreground).toMatch(/^#[0-9a-f]{6}$/i);
      expect(theme.cursor).toMatch(/^#[0-9a-f]{6}$/i);
      expect(theme.extendedAnsi).toHaveLength(240);
      backgrounds.add(theme.background!);
    }
    expect(backgrounds.size).toBe(TERMINAL_THEME_OPTIONS.length);
  });

  it("updates an existing terminal without replacing its instance", () => {
    const target = { options: {} };
    const themeId = applyTerminalPresentation(target, {
      term: "xterm-256color",
      rows: 32,
      cols: 120,
      fontFamily: "Roboto Mono, monospace",
      fontSize: 15,
      scrollback: 50_000,
      theme: "portmate-light",
      backgroundOpacity: 55,
    });

    expect(themeId).toBe("portmate-light");
    expect(target.options).toMatchObject({
      fontFamily: "Roboto Mono, monospace",
      fontSize: 15,
      scrollback: 50_000,
      theme: { background: "rgba(247, 248, 250, 0.55)", foreground: "#202630" },
    });
  });

  it("keeps opaque themes stable and clamps transparent backgrounds", () => {
    expect(terminalTheme("graphite", 100).background).toBe("#171717");
    expect(terminalTheme("graphite", 0).background).toBe("rgba(23, 23, 23, 0.2)");
  });
});
