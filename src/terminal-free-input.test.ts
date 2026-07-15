import { describe, expect, it } from "vitest";
import {
  createTerminalFreeInputPayload,
  cutTerminalFreeInputRange,
  MAX_TERMINAL_FREE_INPUT_CHARACTERS,
  normalizeTerminalFreeInput,
  requestTerminalFreeInput,
  terminalFreeInputCharacterCount,
  TERMINAL_FREE_INPUT_REQUEST_EVENT,
} from "./terminal-free-input";

describe("terminal free input", () => {
  it("normalizes edited lines into one terminal submission", () => {
    expect(createTerminalFreeInputPayload("printf one\nprintf two\r\nprintf three\r")).toBe("printf one\rprintf two\rprintf three\r\r");
    expect(createTerminalFreeInputPayload("  ")).toBe("  \r");
    expect(createTerminalFreeInputPayload("")).toBeNull();
  });

  it("bounds drafts by Unicode characters without splitting emoji", () => {
    const value = `${"a".repeat(MAX_TERMINAL_FREE_INPUT_CHARACTERS - 1)}😀tail`;
    const normalized = normalizeTerminalFreeInput(value);
    expect(terminalFreeInputCharacterCount(normalized)).toBe(MAX_TERMINAL_FREE_INPUT_CHARACTERS);
    expect(normalized.endsWith("😀")).toBe(true);
  });

  it("cuts only the selected editable range", () => {
    expect(cutTerminalFreeInputRange("alpha beta", 6, 10)).toEqual({
      value: "alpha ",
      cutText: "beta",
      caret: 6,
    });
    expect(cutTerminalFreeInputRange("alpha", -10, 99)).toEqual({
      value: "",
      cutText: "alpha",
      caret: 0,
    });
  });

  it("dispatches the shared menu request event", () => {
    const events: Event[] = [];
    expect(requestTerminalFreeInput({ dispatchEvent: (event) => {
      events.push(event);
      return true;
    } })).toBe(true);
    expect(events).toHaveLength(1);
    expect(events[0].type).toBe(TERMINAL_FREE_INPUT_REQUEST_EVENT);
  });
});
