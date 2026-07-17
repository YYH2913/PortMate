import { describe, expect, it } from "vitest";
import {
  defaultWorkspacePanelVisibility,
  isWorkspaceFocusModeShortcut,
  normalizeWorkspacePanelVisibility,
  resolveWorkspacePanelVisibility,
  setWorkspacePanelVisibility,
  toggleWorkspacePanelVisibility,
} from "./workspace-panel-state";

describe("workspace panel state", () => {
  it("uses compact defaults for missing or invalid state", () => {
    expect(normalizeWorkspacePanelVisibility(null)).toEqual(defaultWorkspacePanelVisibility);
    expect(normalizeWorkspacePanelVisibility({ version: 3, panels: { explorer: false } })).toEqual(defaultWorkspacePanelVisibility);
    expect(defaultWorkspacePanelVisibility).toEqual({
      explorer: true,
      fileManager: false,
      history: false,
      sender: false,
      statusBar: true,
    });
  });

  it("migrates the old all-visible default without overwriting customized v1 layouts", () => {
    expect(normalizeWorkspacePanelVisibility({
      version: 1,
      panels: { explorer: true, fileManager: true, sessions: true, history: true, sender: true, statusBar: true },
    })).toEqual(defaultWorkspacePanelVisibility);
    expect(normalizeWorkspacePanelVisibility({
      version: 2,
      panels: { explorer: false, sender: true },
    })).toEqual({ ...defaultWorkspacePanelVisibility, explorer: false, sender: true });
  });

  it("restores v1 snapshots and repairs invalid fields independently", () => {
    expect(normalizeWorkspacePanelVisibility({
      version: 1,
      panels: { explorer: false, fileManager: "no", sender: false, statusBar: false },
    })).toEqual({
      explorer: false,
      fileManager: true,
      history: true,
      sender: false,
      statusBar: false,
    });
  });

  it("supports legacy direct objects and immutable updates", () => {
    const initial = normalizeWorkspacePanelVisibility({ history: false });
    const shown = setWorkspacePanelVisibility(initial, "history", true);
    expect(shown).not.toBe(initial);
    expect(shown.history).toBe(true);
    expect(setWorkspacePanelVisibility(shown, "history", true)).toBe(shown);
    expect(toggleWorkspacePanelVisibility(shown, "sender")).toEqual({ ...shown, sender: false });
  });

  it("derives focus mode without changing the saved panel choices", () => {
    const current = { ...defaultWorkspacePanelVisibility, fileManager: false };
    expect(resolveWorkspacePanelVisibility(current, false)).toBe(current);
    expect(resolveWorkspacePanelVisibility(current, true)).toEqual({
      explorer: false,
      fileManager: false,
      history: false,
      sender: false,
      statusBar: false,
    });
    expect(resolveWorkspacePanelVisibility(current, true, true).statusBar).toBe(true);
    expect(current).toEqual({ ...defaultWorkspacePanelVisibility, fileManager: false });
  });

  it("matches only the exact WindTerm focus-mode shortcut", () => {
    const event = { altKey: true, code: "Enter", ctrlKey: false, metaKey: false, shiftKey: false };
    expect(isWorkspaceFocusModeShortcut(event)).toBe(true);
    expect(isWorkspaceFocusModeShortcut({ ...event, altKey: false })).toBe(false);
    expect(isWorkspaceFocusModeShortcut({ ...event, shiftKey: true })).toBe(false);
    expect(isWorkspaceFocusModeShortcut({ ...event, code: "NumpadEnter" })).toBe(false);
  });
});
