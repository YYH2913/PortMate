import { describe, expect, it } from "vitest";
import {
  emptyTerminalKeySequenceState,
  normalizeTerminalKeyMode,
  resolveTerminalKeyModeEvent,
  terminalKeyModeCursorStyle,
  terminalKeyModeLabel,
  terminalKeyModeShortcutHint,
  terminalLocalNavigationStartRow,
  toggleTerminalInsertNormalMode,
  toggleTerminalRemoteLocalMode,
} from "./terminal-key-mode";

const key = (value: string, overrides: Partial<KeyboardEvent> = {}) => ({
  key: value,
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  shiftKey: false,
  isComposing: false,
  ...overrides,
});

describe("terminal key modes", () => {
  it("passes serial Escape through and requires Shift+Escape for local Normal", () => {
    const serial = { serial: true };
    const resolve = (value: string, overrides: Partial<KeyboardEvent> = {}) => resolveTerminalKeyModeEvent("remote", key(value, overrides), emptyTerminalKeySequenceState(), serial);
    expect(resolve("Escape").handled).toBe(false);
    expect(resolve("Escape").nextMode).toBeUndefined();
    expect(resolve("Escape", { shiftKey: true })).toMatchObject({ handled: true, nextMode: "command" });
    for (const value of ["r", "o", "t", "Enter", "Backspace"]) expect(resolve(value).handled).toBe(false);
    expect(resolve("Escape", { ctrlKey: true })).toMatchObject({ handled: false });
    expect(resolve("Escape", { altKey: true })).toMatchObject({ handled: false });
    expect(resolve("Escape", { isComposing: true })).toMatchObject({ handled: false });
    expect(resolveTerminalKeyModeEvent("command", key("i"), emptyTerminalKeySequenceState(), serial)).toMatchObject({ nextMode: "remote" });
    expect(terminalKeyModeShortcutHint(true)).toContain("Shift+Esc");
    expect(terminalKeyModeShortcutHint(false)).toContain("Esc / i");
  });

  it("rebases stale local navigation after boot output without overriding visible positions", () => {
    expect(terminalLocalNavigationStartRow(3, 139, 44, 182)).toBe(182);
    expect(terminalLocalNavigationStartRow(148, 139, 44, 182)).toBe(148);
    expect(terminalLocalNavigationStartRow(182, 20, 44, 182)).toBe(20);
    expect(terminalLocalNavigationStartRow(70, 20, 44, 31)).toBe(31);
    expect(terminalLocalNavigationStartRow(30, 0, 24, 5)).toBe(5);
  });
  it("normalizes persisted values and exposes stable labels", () => {
    expect(normalizeTerminalKeyMode("command")).toBe("command");
    expect(normalizeTerminalKeyMode("REMOTE")).toBe("remote");
    expect(normalizeTerminalKeyMode(null)).toBe("remote");
    expect(terminalKeyModeLabel("remote")).toBe("Insert 模式");
    expect(terminalKeyModeLabel("command")).toBe("Normal 模式");
    expect(toggleTerminalRemoteLocalMode("remote")).toBe("local");
    expect(toggleTerminalRemoteLocalMode("command")).toBe("remote");
    expect(toggleTerminalInsertNormalMode("remote")).toBe("command");
    expect(toggleTerminalInsertNormalMode("local")).toBe("remote");
    expect(terminalKeyModeCursorStyle("remote")).toBe("bar");
    expect(terminalKeyModeCursorStyle("command")).toBe("block");
    expect(terminalKeyModeCursorStyle("normal")).toBe("underline");
  });

  it("leaves remote input untouched except for the WindTerm mode shortcut", () => {
    expect(resolveTerminalKeyModeEvent("remote", key("x")).handled).toBe(false);
    expect(resolveTerminalKeyModeEvent("remote", key("Enter", { ctrlKey: true }))).toMatchObject({
      handled: true,
      nextMode: "local",
    });
    expect(resolveTerminalKeyModeEvent("local", key("Enter", { ctrlKey: true }))).toMatchObject({ nextMode: "remote" });
  });

  it("switches Insert and Normal without producing accidental remote input", () => {
    expect(resolveTerminalKeyModeEvent("remote", key("Escape"))).toMatchObject({ handled: true, nextMode: "command" });
    expect(resolveTerminalKeyModeEvent("normal", key("a"))).toMatchObject({ handled: true });
    expect(resolveTerminalKeyModeEvent("normal", key("Escape"))).toMatchObject({ nextMode: "command" });
    expect(resolveTerminalKeyModeEvent("command", key("i"))).toMatchObject({ nextMode: "remote" });
    expect(resolveTerminalKeyModeEvent("local", key("i"))).toMatchObject({ nextMode: "remote" });
    expect(resolveTerminalKeyModeEvent("command", key("Enter", { ctrlKey: true }))).toMatchObject({ nextMode: "remote" });
  });

  it("parses count prefixes and Vim-style local navigation", () => {
    const first = resolveTerminalKeyModeEvent("local", key("3"));
    expect(first).toMatchObject({ handled: true, state: { count: "3" } });
    const second = resolveTerminalKeyModeEvent("local", key("2"), first.state);
    const movement = resolveTerminalKeyModeEvent("local", key("j"), second.state);
    expect(movement).toMatchObject({ command: "move-down", count: 32, state: emptyTerminalKeySequenceState() });
    expect(resolveTerminalKeyModeEvent("local", key("0"))).toMatchObject({ command: "line-start", count: 1 });
  });

  it("supports document, page, search and visual-selection commands", () => {
    const prefix = resolveTerminalKeyModeEvent("command", key("g"));
    expect(resolveTerminalKeyModeEvent("command", key("g"), prefix.state)).toMatchObject({ command: "document-start" });
    expect(resolveTerminalKeyModeEvent("command", key("G"))).toMatchObject({ command: "document-end" });
    expect(resolveTerminalKeyModeEvent("command", key("f", { ctrlKey: true }))).toMatchObject({ command: "page-down" });
    expect(resolveTerminalKeyModeEvent("local", key("/"))).toMatchObject({ command: "open-search" });
    expect(resolveTerminalKeyModeEvent("local", key("V"))).toMatchObject({ command: "toggle-line-selection" });
    expect(resolveTerminalKeyModeEvent("local", key("Escape"))).toMatchObject({ command: "clear-selection" });
  });

  it("consumes unmapped and composing keys in every local mode", () => {
    expect(resolveTerminalKeyModeEvent("local", key("z"))).toMatchObject({ handled: true, command: undefined });
    expect(resolveTerminalKeyModeEvent("command", key("Process", { isComposing: true }))).toMatchObject({ handled: true });
    expect(resolveTerminalKeyModeEvent("remote", key("Process", { isComposing: true }))).toMatchObject({ handled: false });
  });
});
