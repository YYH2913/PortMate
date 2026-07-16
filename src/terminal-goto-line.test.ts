import { describe, expect, it } from "vitest";
import {
  MAX_TERMINAL_GOTO_LINE_QUERY_LENGTH,
  resolveTerminalGotoLine,
  terminalGotoCurrentLine,
  terminalGotoLineStatus,
  terminalGotoViewportLine,
} from "./terminal-goto-line";
import { requestTerminalGotoLine, TERMINAL_GOTO_LINE_REQUEST_EVENT } from "./terminal-goto-line-event";

describe("terminal goto line", () => {
  it("resolves absolute and relative lines from the current viewport line", () => {
    expect(resolveTerminalGotoLine("120", 80, 200)).toEqual({ kind: "valid", targetLine: 120, relative: false });
    expect(resolveTerminalGotoLine("+20", 80, 200)).toEqual({ kind: "valid", targetLine: 100, relative: true });
    expect(resolveTerminalGotoLine("-10", 80, 200)).toEqual({ kind: "valid", targetLine: 70, relative: true });
    expect(resolveTerminalGotoLine(" +0 ", 80, 200)).toEqual({ kind: "valid", targetLine: 80, relative: true });
  });

  it("distinguishes empty, malformed, unsafe, and out-of-range input", () => {
    expect(resolveTerminalGotoLine("", 10, 20)).toEqual({ kind: "empty" });
    expect(resolveTerminalGotoLine("1.5", 10, 20)).toEqual({ kind: "invalid" });
    expect(resolveTerminalGotoLine("--1", 10, 20)).toEqual({ kind: "invalid" });
    expect(resolveTerminalGotoLine("9007199254740993", 10, 20)).toEqual({ kind: "invalid" });
    expect(resolveTerminalGotoLine("0", 10, 20)).toEqual({ kind: "out-of-range" });
    expect(resolveTerminalGotoLine("+11", 10, 20)).toEqual({ kind: "out-of-range" });
    expect(resolveTerminalGotoLine("-10", 10, 20)).toEqual({ kind: "out-of-range" });
    expect(MAX_TERMINAL_GOTO_LINE_QUERY_LENGTH).toBeGreaterThanOrEqual(16);
  });

  it("centers targets while clamping the first and final viewports", () => {
    expect(terminalGotoViewportLine(1, 20, 100)).toBe(0);
    expect(terminalGotoViewportLine(50, 20, 100)).toBe(39);
    expect(terminalGotoViewportLine(100, 20, 100)).toBe(80);
    expect(terminalGotoViewportLine(10, 24, 10)).toBe(0);
    expect(terminalGotoCurrentLine(40, 20, 100)).toBe(51);
    expect(terminalGotoCurrentLine(95, 20, 100)).toBe(100);
  });

  it("formats current, valid, invalid, and range status", () => {
    expect(terminalGotoLineStatus({ kind: "empty" }, 42, 100)).toBe("当前 42 / 共 100");
    expect(terminalGotoLineStatus({ kind: "valid", targetLine: 64, relative: true }, 42, 100)).toBe("目标 64 / 共 100");
    expect(terminalGotoLineStatus({ kind: "invalid" }, 42, 100)).toBe("请输入行号");
    expect(terminalGotoLineStatus({ kind: "out-of-range" }, 42, 100)).toBe("范围 1..100");
  });

  it("dispatches the shared focused-terminal request event", () => {
    const events: Event[] = [];
    expect(requestTerminalGotoLine({ dispatchEvent: (event) => {
      events.push(event);
      return true;
    } })).toBe(true);
    expect(events).toHaveLength(1);
    expect(events[0].type).toBe(TERMINAL_GOTO_LINE_REQUEST_EVENT);
  });
});
