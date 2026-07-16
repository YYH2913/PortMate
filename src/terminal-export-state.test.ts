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

describe("terminal text export", () => {
  it("joins wrapped rows, preserves blank logical lines, and trims trailing rows", () => {
    expect(extractTerminalBufferText(buffer([
      { text: "hello " },
      { text: "world", wrapped: true },
      { text: "" },
      { text: "next" },
      { text: "" },
      { text: "   " },
    ]))).toEqual({
      ok: true,
      text: "helloworld\n\nnext",
      bytes: 16,
      logicalLines: 3,
    });
  });

  it("counts UTF-8 bytes rather than UTF-16 code units", () => {
    expect(extractTerminalSelectionText("终端", 6)).toEqual({ ok: true, text: "终端", bytes: 6, logicalLines: 1 });
    expect(extractTerminalSelectionText("终端", 5)).toEqual({ ok: false, reason: "too-large", bytes: 6 });
  });

  it("counts a wrapped first row when scrollback starts inside a logical line", () => {
    expect(extractTerminalBufferText(buffer([
      { text: "continued", wrapped: true },
      { text: "tail" },
    ]))).toEqual({ ok: true, text: "continued\ntail", bytes: 14, logicalLines: 2 });
  });

  it("rejects empty buffers and selections without rejecting whitespace selections", () => {
    expect(extractTerminalBufferText(buffer([{ text: "" }, { text: " " }]))).toEqual({ ok: false, reason: "empty", bytes: 0 });
    expect(extractTerminalSelectionText("")).toEqual({ ok: false, reason: "empty", bytes: 0 });
    expect(extractTerminalSelectionText(" \n")).toEqual({ ok: true, text: " \n", bytes: 2, logicalLines: 2 });
  });

  it("stops before returning a buffer larger than the byte limit", () => {
    expect(extractTerminalBufferText(buffer([{ text: "abcd" }, { text: "efgh" }]), 8))
      .toEqual({ ok: false, reason: "too-large", bytes: 9 });
  });
});
