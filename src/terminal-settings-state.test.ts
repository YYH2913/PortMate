import { describe, expect, it } from "vitest";
import type { SessionSummary } from "./types";
import {
  MAX_TERMINAL_STARTUP_SESSION_ID_CHARACTERS,
  normalizeTerminalStartupSessionIds,
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
});

function session(id: string, name: string, kind: string): SessionSummary {
  return { profile: { id, name, kind } } as SessionSummary;
}
