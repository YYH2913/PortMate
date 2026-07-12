import { describe, expect, it } from "vitest";
import { reconcileWorkspaceSnapshot, resolveStartupSessionIds, sanitizeWorkspaceSnapshot } from "./workspace-state";

describe("workspace snapshots", () => {
  it("sanitizes malformed fields while preserving duplicate pane bindings", () => {
    expect(sanitizeWorkspaceSnapshot({
      version: 99,
      layout: "vertical",
      paneIds: ["a", "a", "b", "c", "d", "e", 7],
      activeId: "b",
      tabColors: { a: "#AABBCC", b: "bad", "": "#112233" },
    })).toEqual({
      version: 1,
      layout: "vertical",
      paneIds: ["a", "a", "b", "c"],
      activeId: "b",
      tabColors: { a: "#AABBCC" },
    });
  });

  it("falls back to a single layout without two panes", () => {
    expect(sanitizeWorkspaceSnapshot({ layout: "horizontal", paneIds: ["a"] })).toMatchObject({
      layout: "single",
      paneIds: [],
    });
  });

  it("reconciles stale panes, active session, and colors", () => {
    const snapshot = sanitizeWorkspaceSnapshot({
      layout: "horizontal",
      paneIds: ["missing", "b", "c"],
      activeId: "missing",
      tabColors: { missing: "#111111", b: "#222222" },
    });
    expect(reconcileWorkspaceSnapshot(snapshot, ["a", "b", "c"])).toEqual({
      version: 1,
      layout: "horizontal",
      paneIds: ["b", "c"],
      activeId: "b",
      tabColors: { b: "#222222" },
    });
  });

  it("collapses a split when only one saved pane remains", () => {
    const snapshot = sanitizeWorkspaceSnapshot({
      layout: "vertical",
      paneIds: ["a", "b"],
      activeId: "b",
    });
    expect(reconcileWorkspaceSnapshot(snapshot, ["b"])).toMatchObject({
      layout: "single",
      paneIds: [],
      activeId: "b",
    });
  });

  it("resolves last workspace panes and configured startup sessions", () => {
    const workspace = sanitizeWorkspaceSnapshot({
      layout: "horizontal",
      paneIds: ["a", "b"],
      activeId: "a",
    });
    expect(resolveStartupSessionIds("last", [], workspace, ["a", "b", "c"])).toEqual(["a", "b"]);
    expect(resolveStartupSessionIds("specific", ["c", "missing", "c", "a"], workspace, ["a", "b", "c"])).toEqual(["c", "a"]);
    expect(resolveStartupSessionIds("none", ["a"], workspace, ["a"])).toEqual([]);
  });

  it("keeps two views of the same session but connects it once", () => {
    const workspace = sanitizeWorkspaceSnapshot({
      layout: "horizontal",
      paneIds: ["a", "a"],
      activeId: "a",
    });
    expect(reconcileWorkspaceSnapshot(workspace, ["a"])).toMatchObject({
      layout: "horizontal",
      paneIds: ["a", "a"],
    });
    expect(resolveStartupSessionIds("last", [], workspace, ["a"])).toEqual(["a"]);
  });
});
