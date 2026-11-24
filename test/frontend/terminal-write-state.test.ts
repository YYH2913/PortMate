import { describe, expect, it } from "vitest";
import {
  isStandaloneLineErase,
  isTerminalCursorMotionOnly,
  terminalPayloadMayMoveRows,
  terminalPayloadMayOverwriteCursorLine,
  terminalWriteHasOutput,
} from "../../src/terminal-write-state";

describe("terminal visual write admission", () => {
  it("does not inspect or render outgoing captures and empty acknowledgements", () => {
    expect(terminalWriteHasOutput({ direction: "outbound", text: "x".repeat(65536) })).toBe(false);
    expect(terminalWriteHasOutput({ direction: "outbound", text: "\x1b[2J" }, new Uint8Array([27, 91, 50, 74]))).toBe(false);
    for (const direction of ["inbound", "system"] as const) {
      expect(terminalWriteHasOutput({ direction, text: "" })).toBe(false);
      expect(terminalWriteHasOutput({ direction, text: null })).toBe(false);
    }
  });
  it("keeps incoming binary/control output and gives raw bytes precedence over text", () => {
    expect(terminalWriteHasOutput({ direction: "inbound", text: "" }, new Uint8Array([0, 0x80, 255]))).toBe(true);
    expect(terminalWriteHasOutput({ direction: "inbound", text: "\x1b[?1049h" })).toBe(true);
    expect(terminalWriteHasOutput({ direction: "system", text: "diagnostic" })).toBe(true);
    expect(terminalWriteHasOutput({ direction: "inbound", text: "not actual raw output" }, new Uint8Array())).toBe(false);
  });
  it("narrows redraws only for a complete, isolated EL on the actual wire", () => {
    const encode = (text: string) => new TextEncoder().encode(text);
    for (const text of ["\x1b[K", "\b\x1b[K"]) {
      expect(isStandaloneLineErase(text)).toBe(true);
      expect(isStandaloneLineErase("decoded preview", encode(text))).toBe(true);
    }
    for (const text of ["", "\b", "\x1b[", "K", "\x1b[2J", "\x1b[K\r\nnext", "\x1b[H\x1b[K", "\b\x1b[K\x1b[K"]) {
      expect(isStandaloneLineErase(text)).toBe(false);
      expect(isStandaloneLineErase("\b\x1b[K", encode(text))).toBe(false);
    }
  });
  it("detects row movement and in-place overwrite controls from raw bytes or decoded text", () => {
    const encode = (text: string) => new TextEncoder().encode(text);
    for (const text of ["\n", "\r", "\f", "\v", "\x1b[K", "line\r\n"]) {
      expect(terminalPayloadMayMoveRows(text)).toBe(true);
      expect(terminalPayloadMayMoveRows("", encode(text))).toBe(true);
    }
    for (const text of ["", "plain text", "status", "\t\b "]) {
      expect(terminalPayloadMayMoveRows(text)).toBe(false);
      expect(terminalPayloadMayMoveRows("\r\n", encode(text))).toBe(false);
    }
    for (const text of ["\rProgress: 50%", "\x1b[1GProgress: 50%", "\x1b[K"]) {
      expect(terminalPayloadMayOverwriteCursorLine(text)).toBe(true);
      expect(terminalPayloadMayOverwriteCursorLine("", encode(text))).toBe(true);
    }
    for (const text of ["", "status", "line1\nline2", "\b \b"]) {
      expect(terminalPayloadMayOverwriteCursorLine(text)).toBe(false);
      expect(terminalPayloadMayOverwriteCursorLine("\r", encode(text))).toBe(false);
    }
  });

  it("distinguishes cursor-only CR/LF packets from writes that can change a row", () => {
    const encode = (text: string) => new TextEncoder().encode(text);
    expect(isTerminalCursorMotionOnly("\r\n")).toBe(true);
    expect(isTerminalCursorMotionOnly("ignored preview", encode("\n\r"))).toBe(true);
    expect(isTerminalCursorMotionOnly("\r\ntext")).toBe(false);
    expect(isTerminalCursorMotionOnly("\r\n", encode("\r\ntext"))).toBe(false);
    expect(isTerminalCursorMotionOnly("\x1b[2K")).toBe(false);
  });
});
