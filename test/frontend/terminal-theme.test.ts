import { describe, expect, it } from "vitest";
import {
  applyTerminalPresentation,
  DEFAULT_TERMINAL_THEME,
  normalizeTerminalTheme,
  terminalTheme,
  TERMINAL_THEME_OPTIONS,
} from "../../src/terminal-theme";

import { terminalSemanticPalettes } from "../../src/terminal-semantic-palette";

function luminance(hex: string) {
  const [r, g, b] = hex.slice(1).match(/../g)!.map(pair => {
    const n = parseInt(pair, 16) / 255;
    return n <= 0.04045 ? n / 12.92 : ((n + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

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

  it("keeps every semantic foreground legible on its opaque terminal background", () => {
    for (const { value } of TERMINAL_THEME_OPTIONS) {
      const bg = luminance(terminalTheme(value).background!);
      const palette = terminalSemanticPalettes[value];
      for (const [role, color] of Object.entries(palette)) {
        const fg = luminance(color);
        expect((Math.max(bg, fg) + 0.05) / (Math.min(bg, fg) + 0.05), `${value}:${role}`).toBeGreaterThanOrEqual(4.5);
      }
      expect(palette.operator).not.toBe(palette.error);
      expect(palette.string).not.toBe(palette.warning);
    }
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
