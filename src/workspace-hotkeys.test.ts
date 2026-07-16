import { describe, expect, it } from "vitest";
import {
  defaultWorkspaceKeymap,
  formatWorkspaceKeyBinding,
  normalizeWorkspaceKeymap,
  resolveWorkspaceHotkey,
  resolveWorkspaceHotkeySequence,
  workspaceKeyBindingFromEvent,
  workspaceKeymapConflicts,
} from "./workspace-hotkeys";

const baseInput = { altKey: true, ctrlKey: false, metaKey: false, shiftKey: false };

describe("workspace hotkeys", () => {
  it.each([
    ["ArrowUp", "up"],
    ["ArrowDown", "down"],
    ["ArrowLeft", "left"],
    ["ArrowRight", "right"],
  ])("maps Alt+%s to %s pane focus", (code, direction) => {
    expect(resolveWorkspaceHotkey({ ...baseInput, code }, 3)).toEqual({ kind: "focus", direction });
  });

  it.each([
    ["Minus", false, "horizontal", "second"],
    ["Minus", true, "horizontal", "first"],
    ["Backslash", false, "vertical", "second"],
    ["Backslash", true, "vertical", "first"],
  ])("maps split key %s shift=%s", (code, shiftKey, direction, placement) => {
    expect(resolveWorkspaceHotkey({ ...baseInput, code, shiftKey }, 1)).toEqual({ kind: "split", direction, placement });
  });

  it("closes or zooms only a multi-pane workspace and rejects conflicting modifiers", () => {
    expect(resolveWorkspaceHotkey({ ...baseInput, code: "KeyX" }, 2)).toEqual({ kind: "close" });
    expect(resolveWorkspaceHotkey({ ...baseInput, code: "KeyZ" }, 2)).toEqual({ kind: "zoom" });
    expect(resolveWorkspaceHotkey({ ...baseInput, code: "KeyX" }, 1)).toBeNull();
    expect(resolveWorkspaceHotkey({ ...baseInput, code: "KeyZ" }, 1)).toBeNull();
    expect(resolveWorkspaceHotkey({ ...baseInput, code: "ArrowRight" }, 1)).toBeNull();
    expect(resolveWorkspaceHotkey({ ...baseInput, code: "ArrowRight", shiftKey: true }, 2)).toBeNull();
    expect(resolveWorkspaceHotkey({ ...baseInput, code: "Backslash", ctrlKey: true }, 2)).toBeNull();
    expect(resolveWorkspaceHotkey({ ...baseInput, code: "Backslash", altKey: false }, 2)).toBeNull();
  });

  it("cycles views with WindTerm remote-mode brackets without consuming local modes", () => {
    expect(resolveWorkspaceHotkey({ ...baseInput, code: "BracketLeft" }, 1)).toEqual({ kind: "cycle-view", offset: -1 });
    expect(resolveWorkspaceHotkey({ ...baseInput, code: "BracketRight" }, 1)).toEqual({ kind: "cycle-view", offset: 1 });
    expect(resolveWorkspaceHotkey(
      { ...baseInput, code: "BracketRight" },
      1,
      defaultWorkspaceKeymap,
      { terminalKeyMode: "local" },
    )).toBeNull();
    expect(formatWorkspaceKeyBinding("Alt+BracketLeft")).toBe("Alt + [");
    expect(formatWorkspaceKeyBinding("Alt+BracketRight")).toBe("Alt + ]");
  });

  it("closes and reopens views with WindTerm lifecycle shortcuts in eligible modes", () => {
    const closeInput = { ...baseInput, altKey: false, ctrlKey: true, shiftKey: true, code: "KeyW" };
    const reopenInput = { ...baseInput, altKey: false, ctrlKey: true, shiftKey: true, code: "KeyT" };

    expect(resolveWorkspaceHotkey(closeInput, 1, defaultWorkspaceKeymap, { terminalKeyMode: "remote" }))
      .toEqual({ kind: "view-history", operation: "close" });
    expect(resolveWorkspaceHotkey(closeInput, 1, defaultWorkspaceKeymap, { terminalKeyMode: "local" }))
      .toEqual({ kind: "view-history", operation: "close" });
    expect(resolveWorkspaceHotkey(closeInput, 1, defaultWorkspaceKeymap, { terminalKeyMode: "normal" }))
      .toBeNull();
    expect(resolveWorkspaceHotkey(closeInput, 1, defaultWorkspaceKeymap, { terminalKeyMode: "command" }))
      .toBeNull();
    expect(resolveWorkspaceHotkey(reopenInput, 1, defaultWorkspaceKeymap, { terminalKeyMode: "command" }))
      .toEqual({ kind: "view-history", operation: "reopen" });
    expect(formatWorkspaceKeyBinding("Ctrl+Shift+KeyW")).toBe("Ctrl + Shift + W");
    expect(formatWorkspaceKeyBinding("Ctrl+Shift+KeyT")).toBe("Ctrl + Shift + T");
  });

  it("normalizes stored bindings, preserves explicit disables, and ignores unknown commands", () => {
    const keymap = normalizeWorkspaceKeymap({
      "focus-up": "Shift+Alt+KeyK",
      "focus-down": "",
      "split-left": "invalid",
      unknown: "Alt+KeyU",
    });

    expect(keymap["focus-up"]).toBe("Alt+Shift+KeyK");
    expect(keymap["focus-down"]).toBe("");
    expect(keymap["split-left"]).toBe(defaultWorkspaceKeymap["split-left"]);
    expect(Object.keys(keymap)).toHaveLength(15);
    expect(resolveWorkspaceHotkey({ ...baseInput, code: "ArrowDown" }, 3, keymap)).toBeNull();
  });

  it("detects conflicts and refuses to resolve ambiguous bindings", () => {
    const keymap = {
      ...defaultWorkspaceKeymap,
      "focus-up": "Alt+KeyK",
      "focus-down": "Alt+KeyK",
    };

    expect(workspaceKeymapConflicts(keymap)).toEqual([{
      binding: "Alt+KeyK",
      commandIds: ["focus-up", "focus-down"],
      kind: "duplicate",
    }]);
    expect(resolveWorkspaceHotkey({ ...baseInput, code: "KeyK" }, 3, keymap)).toBeNull();
  });

  it("resolves a custom binding and formats canonical key labels", () => {
    const keymap = { ...defaultWorkspaceKeymap, "zoom-pane": "Ctrl+Shift+KeyP" };

    expect(resolveWorkspaceHotkey({ ...baseInput, altKey: false, ctrlKey: true, shiftKey: true, code: "KeyP" }, 2, keymap)).toEqual({ kind: "zoom" });
    expect(workspaceKeyBindingFromEvent({ ...baseInput, ctrlKey: true, shiftKey: true, code: "ArrowLeft" })).toBe("Ctrl+Alt+Shift+ArrowLeft");
    expect(workspaceKeyBindingFromEvent({ ...baseInput, altKey: false, code: "KeyP" })).toBeNull();
    expect(formatWorkspaceKeyBinding("Ctrl+Alt+Shift+ArrowLeft")).toBe("Ctrl + Alt + Shift + ←");
    expect(formatWorkspaceKeyBinding("")).toBe("未绑定");
  });

  it("normalizes and resolves a two-stroke WindTerm-style chord", () => {
    const keymap = normalizeWorkspaceKeymap({
      ...defaultWorkspaceKeymap,
      "split-down": "Alt+KeyW   Shift+Ctrl+KeyH",
    });

    expect(keymap["split-down"]).toBe("Alt+KeyW Ctrl+Shift+KeyH");
    expect(resolveWorkspaceHotkeySequence({ ...baseInput, code: "KeyW" }, 1, keymap)).toEqual({
      kind: "pending",
      prefix: "Alt+KeyW",
    });
    expect(resolveWorkspaceHotkeySequence(
      { ...baseInput, altKey: false, ctrlKey: true, shiftKey: true, code: "KeyH" },
      1,
      keymap,
      "Alt+KeyW",
    )).toEqual({ kind: "action", action: { kind: "split", direction: "horizontal", placement: "second" } });
    expect(formatWorkspaceKeyBinding(keymap["split-down"])).toBe("Alt + W  →  Ctrl + Shift + H");
  });

  it("opens OneKeys with WindTerm's Ctrl+T Ctrl+K chord", () => {
    expect(resolveWorkspaceHotkeySequence(
      { ...baseInput, altKey: false, ctrlKey: true, code: "KeyT" },
      1,
    )).toEqual({ kind: "pending", prefix: "Ctrl+KeyT" });
    expect(resolveWorkspaceHotkeySequence(
      { ...baseInput, altKey: false, ctrlKey: true, code: "KeyK" },
      1,
      defaultWorkspaceKeymap,
      "Ctrl+KeyT",
    )).toEqual({ kind: "action", action: { kind: "one-keys" } });
  });

  it("rejects bindings longer than two strokes and detects prefix ambiguity", () => {
    const normalized = normalizeWorkspaceKeymap({
      ...defaultWorkspaceKeymap,
      "focus-up": "Alt+KeyW Alt+KeyH Alt+KeyK",
    });
    const keymap = {
      ...defaultWorkspaceKeymap,
      "focus-up": "Alt+KeyW",
      "focus-down": "Alt+KeyW Alt+KeyJ",
    };

    expect(normalized["focus-up"]).toBe(defaultWorkspaceKeymap["focus-up"]);
    expect(workspaceKeymapConflicts(keymap)).toContainEqual({
      binding: "Alt+KeyW",
      commandIds: ["focus-up", "focus-down"],
      kind: "prefix",
    });
    expect(resolveWorkspaceHotkeySequence({ ...baseInput, code: "KeyW" }, 3, keymap)).toEqual({ kind: "none" });
  });

  it("does not start an unavailable multi-pane chord in a single pane", () => {
    const keymap = { ...defaultWorkspaceKeymap, "close-pane": "Alt+KeyW Alt+KeyX" };

    expect(resolveWorkspaceHotkeySequence({ ...baseInput, code: "KeyW" }, 1, keymap)).toEqual({ kind: "none" });
    expect(resolveWorkspaceHotkeySequence({ ...baseInput, code: "KeyW" }, 2, keymap)).toEqual({
      kind: "pending",
      prefix: "Alt+KeyW",
    });
  });

  it("allows different chords to share their first stroke", () => {
    const keymap = {
      ...defaultWorkspaceKeymap,
      "focus-up": "Alt+KeyW Alt+KeyK",
      "focus-down": "Alt+KeyW Alt+KeyJ",
    };

    expect(workspaceKeymapConflicts(keymap)).toEqual([]);
    expect(resolveWorkspaceHotkeySequence({ ...baseInput, code: "KeyW" }, 3, keymap)).toEqual({
      kind: "pending",
      prefix: "Alt+KeyW",
    });
    expect(resolveWorkspaceHotkeySequence({ ...baseInput, code: "KeyJ" }, 3, keymap, "Alt+KeyW"))
      .toEqual({ kind: "action", action: { kind: "focus", direction: "down" } });
  });
});
