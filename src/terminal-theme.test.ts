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
      fontFamily: "Roboto Mono, monospace",
      fontSize: 15,
      scrollback: 50_000,
      theme: "portmate-light",
    });

    expect(themeId).toBe("portmate-light");
    expect(target.options).toMatchObject({
      fontFamily: "Roboto Mono, monospace",
      fontSize: 15,
      scrollback: 50_000,
      theme: { background: "#f7f8fa", foreground: "#202630" },
    });
  });
});
