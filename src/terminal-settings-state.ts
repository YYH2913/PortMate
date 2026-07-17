import type { SessionSummary } from "./types";

export const TERMINAL_STARTUP_SESSION_SLOTS = 4;
export const MAX_TERMINAL_STARTUP_SESSION_ID_CHARACTERS = 256;

export type TerminalStartupSessionOption = {
  value: string;
  label: string;
};

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
