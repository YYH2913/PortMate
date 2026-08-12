import { describe, expect, it } from "vitest";
import { extractTerminalBufferText, extractTerminalSelectionText } from "./terminal-export-state";

function buffer(lines: Array<{ text: string; wrapped?: boolean }>) {
  return {
    length: lines.length,
    getLine: (index: number) => {
      const line = lines[index];
      return line ? {
        isWrapped: Boolean(line.wrapped),
        translateToString: () => line.text.replace(/\s+$/u, ""),
      } : undefined;
    },
  };
}

const timestamps = [
  { line: 0, ts: "2026-08-09T01:02:03.111111789Z" },
  { line: 1, ts: "2026-08-09T01:02:04.222222999Z" },
  { line: 3, ts: "2026-08-09T01:02:05.333333789Z" },
];

describe("terminal text export", () => {
  it("timestamps physical rows, preserves blank rows, and trims trailing rows", () => {
    expect(extractTerminalBufferText(buffer([
      { text: "hello " },
      { text: "world", wrapped: true },
      { text: "" },
      { text: "next" },
      { text: "" },
      { text: "   " },
    ]), timestamps)).toEqual({
      ok: true,
      text: "[2026-08-09T01:02:03.111111Z] hello\n"
        + "[2026-08-09T01:02:04.222222Z] world\n"
        + "[2026-08-09T01:02:04.222222Z] \n"
        + "[2026-08-09T01:02:05.333333Z] next",
      bytes: 137,
      lineCount: 4,
    });
  });

  it("counts UTF-8 bytes rather than UTF-16 code units", () => {
    expect(extractTerminalSelectionText("终端", 6)).toEqual({ ok: true, text: "终端", bytes: 6, lineCount: 1 });
    expect(extractTerminalSelectionText("终端", 5)).toEqual({ ok: false, reason: "too-large", bytes: 6 });
  });

  it("timestamps a wrapped first row when scrollback starts inside a logical line", () => {
    expect(extractTerminalBufferText(buffer([
      { text: "continued", wrapped: true },
      { text: "tail" },
    ]), timestamps)).toEqual({
      ok: true,
      text: "[2026-08-09T01:02:03.111111Z] continued\n[2026-08-09T01:02:04.222222Z] tail",
      bytes: 74,
      lineCount: 2,
    });
  });

  it("rejects empty or untimestamped buffers and selections without rejecting whitespace selections", () => {
    expect(extractTerminalBufferText(buffer([{ text: "" }, { text: " " }]), timestamps)).toEqual({ ok: false, reason: "empty", bytes: 0 });
    expect(extractTerminalBufferText(buffer([{ text: "ready" }]), [])).toEqual({ ok: false, reason: "missing-timestamp", bytes: 0 });
    expect(extractTerminalSelectionText("")).toEqual({ ok: false, reason: "empty", bytes: 0 });
    expect(extractTerminalSelectionText(" \n")).toEqual({ ok: true, text: " \n", bytes: 2, lineCount: 2 });
  });

  it("counts timestamp prefixes toward the byte limit", () => {
    expect(extractTerminalBufferText(buffer([{ text: "abcd" }, { text: "efgh" }]), timestamps, 67))
      .toEqual({ ok: false, reason: "too-large", bytes: 69 });
  });
});
