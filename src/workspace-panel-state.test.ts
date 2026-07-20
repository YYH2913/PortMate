import { describe, expect, it } from "vitest";
import {
  activateWorkspaceDockPanel,
  activeWorkspaceDockPanel,
  defaultWorkspaceDockLayout,
  defaultWorkspaceDockSizes,
  defaultWorkspacePanelVisibility,
  isWorkspaceFocusModeShortcut,
  moveWorkspacePanelToDock,
  normalizeWorkspaceDockLayout,
  normalizeWorkspaceDockSizes,
  normalizeWorkspacePanelVisibility,
  resolveWorkspacePanelVisibility,
  setWorkspaceDockSize,
  setWorkspacePanelVisibility,
  toggleWorkspacePanelVisibility,
  visibleWorkspaceDockPanels,
  workspaceDockEffectiveSize,
  workspaceDockForPanel,
} from "./workspace-panel-state";

describe("workspace panel state", () => {
  it("uses compact defaults for missing or invalid state", () => {
    expect(normalizeWorkspacePanelVisibility(null)).toEqual(defaultWorkspacePanelVisibility);
    expect(normalizeWorkspacePanelVisibility({ version: 6, panels: { explorer: false } })).toEqual(defaultWorkspacePanelVisibility);
    expect(normalizeWorkspaceDockLayout(null)).toEqual(defaultWorkspaceDockLayout);
    expect(normalizeWorkspaceDockSizes(null)).toEqual(defaultWorkspaceDockSizes);
    expect(defaultWorkspacePanelVisibility).toEqual({
      explorer: true,
      fileManager: false,
      history: false,
      sender: false,
      statusBar: true,
    });
  });

  it("migrates the old all-visible default without overwriting customized layouts", () => {
    expect(normalizeWorkspacePanelVisibility({
      version: 1,
      panels: { explorer: true, fileManager: true, sessions: true, history: true, sender: true, statusBar: true },
    })).toEqual(defaultWorkspacePanelVisibility);
    expect(normalizeWorkspacePanelVisibility({
      version: 2,
      panels: { explorer: true, fileManager: true, history: true, sender: false, statusBar: true },
    })).toEqual({
      explorer: true,
      fileManager: true,
      history: true,
      sender: false,
      statusBar: true,
    });
  });

  it("keeps legacy direct snapshots and visibility updates immutable", () => {
    const initial = normalizeWorkspacePanelVisibility({ history: false });
    const shown = setWorkspacePanelVisibility(initial, "history", true);
    expect(shown).not.toBe(initial);
    expect(shown.history).toBe(true);
    expect(setWorkspacePanelVisibility(shown, "history", true)).toBe(shown);
  });

  it("repairs dock order, duplicates, missing panels and invalid active tabs", () => {
    expect(normalizeWorkspaceDockLayout({
      version: 4,
      docks: {
        left: ["sender", "sender", "unknown"],
        right: ["explorer"],
        bottom: [],
        active: { left: "history", right: "explorer", bottom: "sender" },
      },
    })).toEqual({
      left: ["sender", "fileManager"],
      right: ["explorer", "history"],
      bottom: [],
      active: { left: "sender", right: "explorer", bottom: null },
    });
  });

  it("migrates v4 dock layouts and bounds v5 dock sizes", () => {
    expect(normalizeWorkspaceDockSizes({
      version: 4,
      sizes: { left: 400, right: 400, bottom: 400 },
    })).toEqual(defaultWorkspaceDockSizes);
    expect(normalizeWorkspaceDockSizes({
      version: 5,
      sizes: { left: 50, right: 9999, bottom: 244.6 },
    })).toEqual({ left: 200, right: 720, bottom: 245 });
    expect(normalizeWorkspaceDockSizes({
      version: 5,
      sizes: { left: "320", right: Number.NaN, bottom: null },
    })).toEqual(defaultWorkspaceDockSizes);
  });

  it("updates dock sizes immutably and restores content-aware defaults", () => {
    const resized = setWorkspaceDockSize(defaultWorkspaceDockSizes, "left", 412.4);
    expect(resized).not.toBe(defaultWorkspaceDockSizes);
    expect(resized.left).toBe(412);
    expect(setWorkspaceDockSize(resized, "left", 412)).toBe(resized);
    expect(setWorkspaceDockSize(resized, "left", null).left).toBeNull();
    expect(workspaceDockEffectiveSize(defaultWorkspaceDockSizes, "left", "explorer")).toBe(256);
    expect(workspaceDockEffectiveSize(defaultWorkspaceDockSizes, "left", "fileManager")).toBe(360);
    expect(workspaceDockEffectiveSize(defaultWorkspaceDockSizes, "bottom", "fileManager")).toBe(210);
  });

  it("opens panels independently and derives visible tabs per dock", () => {
    const history = setWorkspacePanelVisibility(defaultWorkspacePanelVisibility, "history", true);
    const sender = setWorkspacePanelVisibility(history, "sender", true);
    expect(sender).toEqual({ ...defaultWorkspacePanelVisibility, history: true, sender: true });
    expect(toggleWorkspacePanelVisibility(sender, "history")).toEqual({ ...sender, history: false });
    expect(visibleWorkspaceDockPanels(defaultWorkspaceDockLayout, sender, "left")).toEqual(["explorer"]);
    expect(visibleWorkspaceDockPanels(defaultWorkspaceDockLayout, sender, "right")).toEqual(["history"]);
    expect(visibleWorkspaceDockPanels(defaultWorkspaceDockLayout, sender, "bottom")).toEqual(["sender"]);
  });

  it("moves and activates dock tabs without losing panel identity", () => {
    const moved = moveWorkspacePanelToDock(defaultWorkspaceDockLayout, "sender", "right", 0);
    expect(moved.right).toEqual(["sender", "history"]);
    expect(moved.bottom).toEqual([]);
    expect(moved.active).toEqual({ left: "explorer", right: "sender", bottom: null });
    expect(workspaceDockForPanel(moved, "sender")).toBe("right");
    expect(activateWorkspaceDockPanel(moved, "history").active.right).toBe("history");
    expect(moveWorkspacePanelToDock(defaultWorkspaceDockLayout, "fileManager", "left", 0).left)
      .toEqual(["fileManager", "explorer"]);
  });

  it("falls back to another visible tab when the configured tab is hidden", () => {
    const visibility = {
      ...defaultWorkspacePanelVisibility,
      explorer: false,
      fileManager: true,
    };
    expect(activeWorkspaceDockPanel(defaultWorkspaceDockLayout, visibility, "left")).toBe("fileManager");
    expect(activeWorkspaceDockPanel(defaultWorkspaceDockLayout, visibility, "right")).toBeNull();
  });

  it("derives focus mode without changing saved panel choices", () => {
    const current = { ...defaultWorkspacePanelVisibility, fileManager: true, history: true };
    expect(resolveWorkspacePanelVisibility(current, false)).toBe(current);
    expect(resolveWorkspacePanelVisibility(current, true)).toEqual({
      explorer: false,
      fileManager: false,
      history: false,
      sender: false,
      statusBar: false,
    });
    expect(resolveWorkspacePanelVisibility(current, true, true).statusBar).toBe(true);
    expect(current.fileManager).toBe(true);
  });

  it("matches only the exact WindTerm focus-mode shortcut", () => {
    const event = { altKey: true, code: "Enter", ctrlKey: false, metaKey: false, shiftKey: false };
    expect(isWorkspaceFocusModeShortcut(event)).toBe(true);
    expect(isWorkspaceFocusModeShortcut({ ...event, altKey: false })).toBe(false);
    expect(isWorkspaceFocusModeShortcut({ ...event, shiftKey: true })).toBe(false);
    expect(isWorkspaceFocusModeShortcut({ ...event, code: "NumpadEnter" })).toBe(false);
  });
});
