import { describe, expect, it } from "vitest";
import { resolveWorkspaceHotkey } from "./workspace-hotkeys";

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

  it("closes only a multi-pane workspace and rejects conflicting modifiers", () => {
    expect(resolveWorkspaceHotkey({ ...baseInput, code: "KeyX" }, 2)).toEqual({ kind: "close" });
    expect(resolveWorkspaceHotkey({ ...baseInput, code: "KeyX" }, 1)).toBeNull();
    expect(resolveWorkspaceHotkey({ ...baseInput, code: "ArrowRight" }, 1)).toBeNull();
    expect(resolveWorkspaceHotkey({ ...baseInput, code: "ArrowRight", shiftKey: true }, 2)).toBeNull();
    expect(resolveWorkspaceHotkey({ ...baseInput, code: "Backslash", ctrlKey: true }, 2)).toBeNull();
    expect(resolveWorkspaceHotkey({ ...baseInput, code: "Backslash", altKey: false }, 2)).toBeNull();
  });
});
