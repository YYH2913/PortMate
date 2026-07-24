import type { SessionProfile, SessionSummary } from "./types";

export const TERMINAL_STARTUP_SESSION_SLOTS = 4;
export const MAX_TERMINAL_STARTUP_SESSION_ID_CHARACTERS = 256;
export const MAX_TERMINAL_NAME_BYTES = 64;
export const MAX_TERMINAL_FONT_FAMILY_CHARACTERS = 256;
export const TERMINAL_PROFILE_BOUNDS = {
  rows: { min: 1, max: 512, fallback: 32 },
  cols: { min: 1, max: 1024, fallback: 120 },
  scrollback: { min: 0, max: 10_000_000, fallback: 200_000 },
  fontSize: { min: 6, max: 72, fallback: 13 },
  backgroundOpacity: { min: 20, max: 100, fallback: 100 },
} as const;

const DEFAULT_TERMINAL_NAME = "xterm-256color";
const DEFAULT_TERMINAL_FONT_FAMILY = "Roboto Mono, JetBrains Mono, monospace";
const terminalNamePattern = /^[A-Za-z0-9][A-Za-z0-9._+-]*$/;

export type TerminalStartupSessionOption = {
  value: string;
  label: string;
};

export function normalizeTerminalProfileSettings(
  value: SessionProfile["terminal"],
): SessionProfile["terminal"] {
  const fontFamily = typeof value.fontFamily === "string" ? value.fontFamily.trim() : "";
  return {
    ...value,
    term: normalizeTerminalName(value.term) ?? DEFAULT_TERMINAL_NAME,
    rows: boundedInteger(value.rows, TERMINAL_PROFILE_BOUNDS.rows),
    cols: boundedInteger(value.cols, TERMINAL_PROFILE_BOUNDS.cols),
    scrollback: boundedInteger(value.scrollback, TERMINAL_PROFILE_BOUNDS.scrollback),
    fontFamily: fontFamily
      && Array.from(fontFamily).length <= MAX_TERMINAL_FONT_FAMILY_CHARACTERS
      && !/[\u0000-\u001f\u007f-\u009f]/.test(fontFamily)
      ? fontFamily
      : DEFAULT_TERMINAL_FONT_FAMILY,
    fontSize: boundedInteger(value.fontSize, TERMINAL_PROFILE_BOUNDS.fontSize),
    backgroundOpacity: normalizeTerminalBackgroundOpacity(value.backgroundOpacity),
  };
}

export function normalizeTerminalName(value: unknown): string | null {
  const term = typeof value === "string" ? value.trim() : "";
  return terminalNamePattern.test(term) && term.length <= MAX_TERMINAL_NAME_BYTES
    ? term
    : null;
}

export function normalizeTerminalBackgroundOpacity(value: unknown): number {
  return boundedInteger(value, TERMINAL_PROFILE_BOUNDS.backgroundOpacity);
}

export function normalizeTerminalStartupSessionIds(value: unknown): string[] {
  const source = Array.isArray(value) ? value : [];
  return Array.from({ length: TERMINAL_STARTUP_SESSION_SLOTS }, (_, index) => {
    const item = source[index];
    return validSessionId(item) ? item : "";
  });
}

export function terminalStartupSessionOptions(
  sessions: readonly SessionSummary[],
  currentValue = "",
): TerminalStartupSessionOption[] {
  const seen = new Set<string>();
  const available = sessions.flatMap((session) => {
    const id = session.profile.id;
    if (!validSessionId(id) || seen.has(id)) return [];
    seen.add(id);
    return [{
      value: id,
      label: `${session.profile.name.trim() || id} · ${session.profile.kind.toUpperCase()}`,
    }];
  });
  const unavailable = validSessionId(currentValue) && !seen.has(currentValue)
    ? [{ value: currentValue, label: `不可用会话 · ${currentValue}` }]
    : [];
  return [{ value: "", label: "未指定" }, ...unavailable, ...available];
}

function validSessionId(value: unknown): value is string {
  return typeof value === "string"
    && Boolean(value)
    && !value.includes("\0")
    && Array.from(value).length <= MAX_TERMINAL_STARTUP_SESSION_ID_CHARACTERS;
}

function boundedInteger(
  value: unknown,
  bounds: { readonly min: number; readonly max: number; readonly fallback: number },
): number {
  const number = typeof value === "number" ? value : Number.NaN;
  if (!Number.isFinite(number)) return bounds.fallback;
  return Math.min(bounds.max, Math.max(bounds.min, Math.trunc(number)));
}
