import { describe, expect, it } from "vitest";
import {
  isTerminalFindShortcut,
  MAX_TERMINAL_SEARCH_QUERY_LENGTH,
  requestTerminalSearch,
  terminalSearchResultLabel,
  terminalSearchSeed,
  TERMINAL_SEARCH_REQUEST_EVENT,
} from "./terminal-search";

describe("terminal search", () => {
  it("recognizes standard and WindTerm-compatible find shortcuts", () => {
    expect(isTerminalFindShortcut({ key: "f", ctrlKey: true, metaKey: false, altKey: false })).toBe(true);
    expect(isTerminalFindShortcut({ key: "F", ctrlKey: false, metaKey: true, altKey: false })).toBe(true);
    expect(isTerminalFindShortcut({ key: "f", ctrlKey: true, metaKey: false, altKey: true })).toBe(false);
    expect(isTerminalFindShortcut({ key: "g", ctrlKey: true, metaKey: false, altKey: false })).toBe(false);
  });

  it("normalizes a bounded single-line seed from the terminal selection", () => {
    expect(terminalSearchSeed("  alpha\r\nbeta  ")).toBe("alpha beta");
    expect(terminalSearchSeed("界".repeat(MAX_TERMINAL_SEARCH_QUERY_LENGTH + 1))).toHaveLength(MAX_TERMINAL_SEARCH_QUERY_LENGTH);
    const emojiSeed = terminalSearchSeed("😀".repeat(MAX_TERMINAL_SEARCH_QUERY_LENGTH));
    expect(emojiSeed).toHaveLength(MAX_TERMINAL_SEARCH_QUERY_LENGTH);
    expect(Array.from(emojiSeed)).toHaveLength(MAX_TERMINAL_SEARCH_QUERY_LENGTH / 2);
  });

  it("formats empty, active, overflow, and invalid search results", () => {
    expect(terminalSearchResultLabel("", null)).toBe("0/0");
    expect(terminalSearchResultLabel("host", { resultIndex: 1, resultCount: 3 })).toBe("2/3");
    expect(terminalSearchResultLabel("host", { resultIndex: -1, resultCount: 1001 })).toBe("1001 个结果");
    expect(terminalSearchResultLabel("[", null, true)).toBe("表达式无效");
  });

  it("dispatches the shared menu request event", () => {
    const events: Event[] = [];
    expect(requestTerminalSearch({ dispatchEvent: (event) => {
      events.push(event);
      return true;
    } })).toBe(true);
    expect(events).toHaveLength(1);
    expect(events[0]).toBeInstanceOf(Event);
    expect(events[0].type).toBe(TERMINAL_SEARCH_REQUEST_EVENT);
  });
});
