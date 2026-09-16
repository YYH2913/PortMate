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
  workspaceDockInsertionIndex,
} from "./workspace-panel-state";
import type { WorkspaceDockLayout } from "./workspace-panel-state";

describe("workspace panel state", () => {
  it("uses compact defaults for missing or invalid state", () => {
    expect(normalizeWorkspacePanelVisibility(null)).toEqual(defaultWorkspacePanelVisibility);
    expect(normalizeWorkspacePanelVisibility({ version: 8, panels: { explorer: false } })).toEqual(defaultWorkspacePanelVisibility);
    expect(normalizeWorkspaceDockLayout(null)).toEqual(defaultWorkspaceDockLayout);
    expect(normalizeWorkspaceDockSizes(null)).toEqual(defaultWorkspaceDockSizes);
    expect(defaultWorkspacePanelVisibility).toEqual({
      explorer: true,
      fileManager: false,
      history: false,
      sysmon: false,
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
      sysmon: false,
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

  it("repairs dock order, duplicates, missing panels and invalid focused views", () => {
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
      right: ["explorer", "history", "sysmon"],
      bottom: [],
      active: { left: "sender", right: "explorer", bottom: null },
    });
  });

  it("migrates v4/v5 dock layouts and bounds v6/v7 dock sizes", () => {
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
    expect(normalizeWorkspaceDockSizes({
      version: 7,
      sizes: { left: 360, right: 280, bottom: 210 },
    })).toEqual({ left: 360, right: 280, bottom: 210 });
  });

  it("updates dock sizes immutably and restores content-aware defaults", () => {
    const resized = setWorkspaceDockSize(defaultWorkspaceDockSizes, "left", 412.4);
    expect(resized).not.toBe(defaultWorkspaceDockSizes);
    expect(resized.left).toBe(412);
    expect(setWorkspaceDockSize(resized, "left", 412)).toBe(resized);
    expect(setWorkspaceDockSize(resized, "left", null).left).toBeNull();
    expect(workspaceDockEffectiveSize(defaultWorkspaceDockSizes, "left", ["explorer"])).toBe(256);
    expect(workspaceDockEffectiveSize(defaultWorkspaceDockSizes, "left", ["explorer", "fileManager"])).toBe(360);
    expect(workspaceDockEffectiveSize(defaultWorkspaceDockSizes, "bottom", ["fileManager"])).toBe(210);
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
    expect(moved.right).toEqual(["sender", "sysmon", "history"]);
    expect(moved.bottom).toEqual([]);
    expect(moved.active).toEqual({ left: "explorer", right: "sender", bottom: null });
    expect(workspaceDockForPanel(moved, "sender")).toBe("right");
    expect(activateWorkspaceDockPanel(moved, "history").active.right).toBe("history");
    expect(moveWorkspacePanelToDock(defaultWorkspaceDockLayout, "fileManager", "left", 0).left)
      .toEqual(["fileManager", "explorer"]);
    expect(moveWorkspacePanelToDock(defaultWorkspaceDockLayout, "explorer", "left", 2).left)
      .toEqual(["fileManager", "explorer"]);
    const reverse = moveWorkspacePanelToDock(defaultWorkspaceDockLayout, "fileManager", "left", 0);
    expect(moveWorkspacePanelToDock(reverse, "fileManager", "left", 2).left)
      .toEqual(["explorer", "fileManager"]);
  });

  it.each([
    [false, 2, ["fileManager", "explorer", "sender", "history", "sysmon"]],
    [true, 3, ["fileManager", "explorer", "history", "sender", "sysmon"]],
  ] as const)("anchors drops to the full dock order despite hidden panels (after=%s)", (after, expectedIndex, expectedOrder) => {
    const layout = dockLayoutWithHiddenTabs();
    const snapshot = structuredClone(layout);
    const visibility = { ...defaultWorkspacePanelVisibility, history: true, sender: true };
    expect(visibleWorkspaceDockPanels(layout, visibility, "left")).toEqual(["explorer", "history"]);
    const targetIndex = workspaceDockInsertionIndex(layout, "left", { panel: "history", after });
    expect(targetIndex).toBe(expectedIndex);
    const moved = moveWorkspacePanelToDock(layout, "sender", "left", targetIndex);
    expect(moved.left).toEqual(expectedOrder);
    expect(moved.bottom).toEqual([]);
    expect(moved.active).toEqual({ left: "sender", right: null, bottom: null });
    expect(visibleWorkspaceDockPanels(moved, visibility, "left")).toEqual(after
      ? ["explorer", "history", "sender"] : ["explorer", "sender", "history"]);
    expect(layout).toEqual(snapshot);
  });

  it.each([
    ["explorer", "sysmon", false, ["fileManager", "history", "explorer", "sysmon"]],
    ["explorer", "sysmon", true, ["fileManager", "history", "sysmon", "explorer"]],
    ["sysmon", "explorer", false, ["fileManager", "sysmon", "explorer", "history"]],
    ["sysmon", "explorer", true, ["fileManager", "explorer", "sysmon", "history"]],
    ["explorer", "explorer", false, ["fileManager", "explorer", "history", "sysmon"]],
    ["explorer", "explorer", true, ["fileManager", "explorer", "history", "sysmon"]],
  ] as const)("reorders %s around %s in the same dock (after=%s) without index drift", (panel, anchor, after, expectedOrder) => {
    const layout = dockLayoutWithHiddenTabs();
    const snapshot = structuredClone(layout);
    const targetIndex = workspaceDockInsertionIndex(layout, "left", { panel: anchor, after });
    const moved = moveWorkspacePanelToDock(layout, panel, "left", targetIndex);
    expect(moved.left).toEqual(expectedOrder);
    expect(moved.right).toEqual([]);
    expect(moved.bottom).toEqual(["sender"]);
    expect(moved.active).toEqual({ left: panel, right: null, bottom: "sender" });
    expect(new Set([...moved.left, ...moved.right, ...moved.bottom]).size).toBe(5);
    expect(layout).toEqual(snapshot);
  });

  it("appends when the drop has no anchor or its anchor no longer belongs to the target dock", () => {
    const layout = dockLayoutWithHiddenTabs();
    for (const anchor of [undefined, { panel: "sender", after: false }, { panel: "sender", after: true }] as const) {
      const targetIndex = workspaceDockInsertionIndex(layout, "left", anchor);
      expect(targetIndex).toBe(4);
      expect(moveWorkspacePanelToDock(layout, "sender", "left", targetIndex).left)
        .toEqual(["fileManager", "explorer", "history", "sysmon", "sender"]);
      expect(moveWorkspacePanelToDock(layout, "explorer", "left", targetIndex).left)
        .toEqual(["fileManager", "history", "sysmon", "explorer"]);
    }
    expect(workspaceDockInsertionIndex(layout, "right", { panel: "history", after: true })).toBe(0);
    const moved = moveWorkspacePanelToDock(layout, "sender", "right", workspaceDockInsertionIndex(layout, "right"));
    expect(moved.right).toEqual(["sender"]);
    expect(moved.bottom).toEqual([]);
    expect(moved.active).toEqual({ left: "explorer", right: "sender", bottom: null });
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
      sysmon: false,
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

function dockLayoutWithHiddenTabs(): WorkspaceDockLayout {
  return {
    left: ["fileManager", "explorer", "history", "sysmon"],
    right: [],
    bottom: ["sender"],
    active: { left: "explorer", right: null, bottom: "sender" },
  };
}
