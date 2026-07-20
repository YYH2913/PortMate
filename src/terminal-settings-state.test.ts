import { describe, expect, it } from "vitest";
import type { SessionSummary } from "./types";
import {
  MAX_TERMINAL_STARTUP_SESSION_ID_CHARACTERS,
  normalizeTerminalProfileSettings,
  normalizeTerminalStartupSessionIds,
  TERMINAL_PROFILE_BOUNDS,
  terminalStartupSessionOptions,
} from "./terminal-settings-state";

describe("terminal settings state", () => {
  it("normalizes exactly four bounded startup session IDs", () => {
    const oversized = "x".repeat(MAX_TERMINAL_STARTUP_SESSION_ID_CHARACTERS + 1);
    expect(normalizeTerminalStartupSessionIds(["edge", "bad\0id", oversized, 42, "ignored"])).toEqual([
      "edge",
      "",
      "",
      "",
    ]);
    expect(normalizeTerminalStartupSessionIds(null)).toEqual(["", "", "", ""]);
  });

  it("uses real profile IDs and preserves a visible unavailable selection", () => {
    const sessions = [
      session("edge", "Edge Router", "ssh"),
      session("uart", "Bench UART", "serial"),
      session("edge", "Duplicate", "ssh"),
    ];
    expect(terminalStartupSessionOptions(sessions, "removed")).toEqual([
      { value: "", label: "未指定" },
      { value: "removed", label: "不可用会话 · removed" },
      { value: "edge", label: "Edge Router · SSH" },
      { value: "uart", label: "Bench UART · SERIAL" },
    ]);
  });

  it("bounds terminal dimensions, scrollback, and font size", () => {
    const normalized = normalizeTerminalProfileSettings({
      term: " xterm-256color ",
      rows: 0,
      cols: 100_000,
      scrollback: Number.POSITIVE_INFINITY,
      fontFamily: " JetBrains Mono, monospace ",
      fontSize: 5.9,
      theme: "portmate-dark",
    });

    expect(normalized).toMatchObject({
      term: "xterm-256color",
      rows: TERMINAL_PROFILE_BOUNDS.rows.min,
      cols: TERMINAL_PROFILE_BOUNDS.cols.max,
      scrollback: TERMINAL_PROFILE_BOUNDS.scrollback.fallback,
      fontFamily: "JetBrains Mono, monospace",
      fontSize: TERMINAL_PROFILE_BOUNDS.fontSize.min,
    });
  });

  it("replaces unsafe terminal and font names with stable defaults", () => {
    const normalized = normalizeTerminalProfileSettings({
      term: "xterm\nmalformed",
      rows: 32,
      cols: 120,
      scrollback: 200_000,
      fontFamily: `monospace\u0000${"x".repeat(300)}`,
      fontSize: 13,
      theme: "portmate-dark",
    });

    expect(normalized.term).toBe("xterm-256color");
    expect(normalized.fontFamily).toBe("Roboto Mono, JetBrains Mono, monospace");
  });
});

function session(id: string, name: string, kind: string): SessionSummary {
  return { profile: { id, name, kind } } as SessionSummary;
}
