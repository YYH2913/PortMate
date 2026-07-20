import type { ITheme } from "@xterm/xterm";
import { normalizeTerminalProfileSettings } from "./terminal-settings-state";
import type { SessionProfile } from "./types";

export const DEFAULT_TERMINAL_THEME = "portmate-dark";

export const TERMINAL_THEME_OPTIONS = [
  { value: "portmate-dark", label: "PortMate 深色" },
  { value: "graphite", label: "石墨" },
  { value: "solarized-dark", label: "Solarized 深色" },
  { value: "portmate-light", label: "PortMate 浅色" },
] as const;

export type TerminalThemeId = typeof TERMINAL_THEME_OPTIONS[number]["value"];

type TerminalPresentationTarget = {
  options: {
    fontFamily?: string;
    fontSize?: number;
    scrollback?: number;
    theme?: ITheme;
  };
};

const extendedAnsi = createXterm256Palette();

const terminalThemes: Record<TerminalThemeId, ITheme> = {
  "portmate-dark": {
    background: "#0d1117",
    foreground: "#d7e1eb",
    cursor: "#5eead4",
    cursorAccent: "#0d1117",
    selectionBackground: "#284457",
    selectionForeground: "#f8fafc",
    black: "#0d1117",
    red: "#f87171",
    green: "#37d67a",
    yellow: "#f4b860",
    blue: "#68a7ff",
    magenta: "#c084fc",
    cyan: "#5eead4",
    white: "#d7e1eb",
    brightBlack: "#6b7280",
    brightRed: "#ff8a8a",
    brightGreen: "#86efac",
    brightYellow: "#fde047",
    brightBlue: "#93c5fd",
    brightMagenta: "#d8b4fe",
    brightCyan: "#67e8f9",
    brightWhite: "#ffffff",
    extendedAnsi,
  },
  graphite: {
    background: "#171717",
    foreground: "#e5e7eb",
    cursor: "#f0b86e",
    cursorAccent: "#171717",
    selectionBackground: "#3f4854",
    selectionForeground: "#ffffff",
    black: "#171717",
    red: "#e06c75",
    green: "#8fb573",
    yellow: "#d6ae5c",
    blue: "#6f9fd8",
    magenta: "#b68acb",
    cyan: "#6fb6b0",
    white: "#d7d9dc",
    brightBlack: "#777d86",
    brightRed: "#f08b92",
    brightGreen: "#add18e",
    brightYellow: "#ebc876",
    brightBlue: "#8eb9e9",
    brightMagenta: "#cfa7df",
    brightCyan: "#8ed0ca",
    brightWhite: "#ffffff",
    extendedAnsi,
  },
  "solarized-dark": {
    background: "#002b36",
    foreground: "#93a1a1",
    cursor: "#b58900",
    cursorAccent: "#002b36",
    selectionBackground: "#174b57",
    selectionForeground: "#fdf6e3",
    black: "#073642",
    red: "#dc322f",
    green: "#859900",
    yellow: "#b58900",
    blue: "#268bd2",
    magenta: "#d33682",
    cyan: "#2aa198",
    white: "#eee8d5",
    brightBlack: "#657b83",
    brightRed: "#cb4b16",
    brightGreen: "#586e75",
    brightYellow: "#657b83",
    brightBlue: "#839496",
    brightMagenta: "#6c71c4",
    brightCyan: "#93a1a1",
    brightWhite: "#fdf6e3",
    extendedAnsi,
  },
  "portmate-light": {
    background: "#f7f8fa",
    foreground: "#202630",
    cursor: "#087f73",
    cursorAccent: "#f7f8fa",
    selectionBackground: "#b9d8e8",
    selectionForeground: "#111827",
    black: "#202630",
    red: "#b4232c",
    green: "#287a46",
    yellow: "#8a5d00",
    blue: "#1d5fa7",
    magenta: "#7a3e9d",
    cyan: "#087f73",
    white: "#d9dde3",
    brightBlack: "#667085",
    brightRed: "#d43b45",
    brightGreen: "#36965a",
    brightYellow: "#a8740b",
    brightBlue: "#347cc5",
    brightMagenta: "#9757b8",
    brightCyan: "#159b8e",
    brightWhite: "#ffffff",
    extendedAnsi,
  },
};

export function normalizeTerminalTheme(value: unknown): TerminalThemeId {
  const candidate = typeof value === "string" ? value.trim() : "";
  return TERMINAL_THEME_OPTIONS.some((option) => option.value === candidate)
    ? candidate as TerminalThemeId
    : DEFAULT_TERMINAL_THEME;
}

export function terminalTheme(value: unknown): ITheme {
  return terminalThemes[normalizeTerminalTheme(value)];
}

export function applyTerminalPresentation(
  target: TerminalPresentationTarget,
  presentation: SessionProfile["terminal"],
): TerminalThemeId {
  const normalized = normalizeTerminalProfileSettings(presentation);
  const themeId = normalizeTerminalTheme(normalized.theme);
  target.options.fontFamily = normalized.fontFamily;
  target.options.fontSize = normalized.fontSize;
  target.options.scrollback = normalized.scrollback;
  target.options.theme = terminalThemes[themeId];
  return themeId;
}

function createXterm256Palette() {
  const toHex = (value: number) => value.toString(16).padStart(2, "0");
  const rgb = (red: number, green: number, blue: number) => `#${toHex(red)}${toHex(green)}${toHex(blue)}`;
  const palette: string[] = [];
  const steps = [0, 95, 135, 175, 215, 255];
  for (const red of steps) {
    for (const green of steps) {
      for (const blue of steps) {
        palette.push(rgb(red, green, blue));
      }
    }
  }
  for (let index = 0; index < 24; index += 1) {
    const level = 8 + index * 10;
    palette.push(rgb(level, level, level));
  }
  return palette;
}
