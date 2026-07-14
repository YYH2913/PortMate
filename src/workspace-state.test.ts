import { describe, expect, it } from "vitest";
import {
  activateWorkspacePaneSession,
  addWorkspacePaneSession,
  MAX_WORKSPACE_DEPTH,
  MAX_WORKSPACE_GROUP_TABS,
  mergeWorkspacePaneGroups,
  findWorkspacePaneInDirection,
  findWorkspacePane,
  reconcileWorkspaceSnapshot,
  removeWorkspacePane,
  removeWorkspacePaneSession,
  replaceWorkspacePaneSession,
  moveWorkspacePaneSession,
  resolveStartupSessionIds,
  sanitizeWorkspaceSnapshot,
  splitWorkspacePane,
  splitWorkspacePaneSessionToGroup,
  swapWorkspacePanes,
  updateWorkspaceSplitRatio,
  workspacePaneLeaves,
} from "./workspace-state";
import type { WorkspaceNode, WorkspaceSplitNode } from "./workspace-state";

describe("workspace snapshots", () => {
  it("migrates flat v1 layouts into equal recursive splits and preserves duplicate bindings", () => {
    const snapshot = sanitizeWorkspaceSnapshot({
      version: 1,
      layout: "vertical",
      paneIds: ["a", "a", "b", "c", "d"],
      activeId: "b",
      tabColors: { a: "#AABBCC", b: "bad", "": "#112233" },
    });

    expect(snapshot.version).toBe(3);
    expect(workspacePaneLeaves(snapshot.root).map((pane) => pane.sessionId)).toEqual(["a", "a", "b", "c"]);
    expect(snapshot.activeId).toBe("b");
    expect(findWorkspacePane(snapshot.root, snapshot.activePaneId)?.sessionId).toBe("b");
    expect(snapshot.tabColors).toEqual({ a: "#AABBCC" });
    expect(snapshot.root).toMatchObject({ kind: "split", direction: "vertical", ratio: 0.25 });
  });

  it("sanitizes malformed v2 trees, duplicate node ids, ratios, and active panes", () => {
    const snapshot = sanitizeWorkspaceSnapshot({
      version: 2,
      root: {
        kind: "split",
        id: "same",
        direction: "bad",
        ratio: 9,
        first: { kind: "pane", id: "same", sessionId: " a\n" },
        second: { kind: "pane", id: "same", sessionId: "b" },
      },
      activePaneId: "missing",
      activeId: "b",
    });
    const panes = workspacePaneLeaves(snapshot.root);

    expect(snapshot.root).toMatchObject({ kind: "split", direction: "horizontal", ratio: 0.85 });
    expect(panes.map((pane) => pane.sessionId)).toEqual(["a", "b"]);
    expect(new Set([snapshot.root?.id, ...panes.map((pane) => pane.id)]).size).toBe(3);
    expect(findWorkspacePane(snapshot.root, snapshot.activePaneId)?.sessionId).toBe("b");
    expect(panes.every((pane) => pane.sessionIds.length === 1 && pane.sessionIds[0] === pane.sessionId)).toBe(true);
  });

  it("sanitizes bounded v3 tab groups", () => {
    const sessionIds = Array.from({ length: MAX_WORKSPACE_GROUP_TABS + 5 }, (_, index) => `session-${index}`);
    const snapshot = sanitizeWorkspaceSnapshot({
      version: 3,
      root: { kind: "pane", id: "group-a", sessionId: "session-2", sessionIds: ["session-0", "session-1", "session-2", "session-1", ...sessionIds.slice(3)] },
      activePaneId: "group-a",
      activeId: "session-2",
    });
    const pane = workspacePaneLeaves(snapshot.root)[0];

    expect(snapshot.version).toBe(3);
    expect(pane.sessionId).toBe("session-2");
    expect(pane.sessionIds).toHaveLength(MAX_WORKSPACE_GROUP_TABS);
    expect(new Set(pane.sessionIds).size).toBe(MAX_WORKSPACE_GROUP_TABS);
  });

  it("activates a persisted view inside the requested group", () => {
    const snapshot = sanitizeWorkspaceSnapshot({
      version: 3,
      root: { kind: "pane", id: "group-a", sessionId: "a", sessionIds: ["a", "b"] },
      activePaneId: "group-a",
      activeId: "b",
    });

    expect(snapshot.activeId).toBe("b");
    expect(findWorkspacePane(snapshot.root, "group-a")?.sessionId).toBe("b");
  });

  it("reconciles stale panes and collapses split branches", () => {
    const snapshot = sanitizeWorkspaceSnapshot({
      version: 1,
      layout: "horizontal",
      paneIds: ["missing", "b", "c"],
      activeId: "missing",
      tabColors: { missing: "#111111", b: "#222222" },
    });
    const reconciled = reconcileWorkspaceSnapshot(snapshot, ["a", "b", "c"]);

    expect(workspacePaneLeaves(reconciled.root).map((pane) => pane.sessionId)).toEqual(["b", "c"]);
    expect(reconciled.activeId).toBe("b");
    expect(reconciled.tabColors).toEqual({ b: "#222222" });
  });

  it("splits nested active panes, replaces bindings, clamps ratios, and collapses on close", () => {
    const initial = sanitizeWorkspaceSnapshot({ version: 1, layout: "single", activeId: "a" });
    const firstPane = workspacePaneLeaves(initial.root)[0];
    const vertical = splitWorkspacePane(initial.root!, firstPane.id, "vertical", "b", "pane-b", "split-v");
    const nested = splitWorkspacePane(vertical, "pane-b", "horizontal", "c", "pane-c", "split-h");
    const resized = updateWorkspaceSplitRatio(nested, "split-h", 0.02)!;
    const replaced = replaceWorkspacePaneSession(resized, "pane-c", "d")!;

    expect(workspacePaneLeaves(replaced).map((pane) => pane.sessionId)).toEqual(["a", "b", "d"]);
    expect((findSplit(replaced, "split-h")?.ratio)).toBe(0.15);
    expect(removeWorkspacePane(replaced, "pane-b")).toMatchObject({
      kind: "split",
      id: "split-v",
      second: { kind: "pane", id: "pane-c", sessionId: "d" },
    });
  });

  it("adds and activates views inside one workspace group", () => {
    const initial = sanitizeWorkspaceSnapshot({ version: 1, layout: "single", activeId: "a" }).root!;
    const paneId = workspacePaneLeaves(initial)[0].id;
    const withTabs = addWorkspacePaneSession(addWorkspacePaneSession(initial, paneId, "b"), paneId, "c")!;
    const activated = activateWorkspacePaneSession(withTabs, paneId, "b")!;
    const pane = findWorkspacePane(activated, paneId)!;

    expect(pane.sessionIds).toEqual(["a", "b", "c"]);
    expect(pane.sessionId).toBe("b");
    expect(activateWorkspacePaneSession(activated, paneId, "missing")).toBe(activated);
  });

  it("removes one view without closing a non-empty group", () => {
    const snapshot = sanitizeWorkspaceSnapshot({
      version: 3,
      root: { kind: "pane", id: "group-a", sessionId: "b", sessionIds: ["a", "b", "c"] },
      activePaneId: "group-a",
      activeId: "b",
    });
    const removed = removeWorkspacePaneSession(snapshot.root, "group-a", "b")!;

    expect(removed).toMatchObject({ kind: "pane", id: "group-a", sessionId: "c", sessionIds: ["a", "c"] });
    expect(removeWorkspacePaneSession(removed, "group-a", "missing")).toBe(removed);
  });

  it("moves the active view across groups and keeps a non-empty source", () => {
    const snapshot = sanitizeWorkspaceSnapshot({
      version: 3,
      root: {
        kind: "split",
        id: "root",
        direction: "vertical",
        ratio: 0.5,
        first: { kind: "pane", id: "group-a", sessionId: "b", sessionIds: ["a", "b"] },
        second: { kind: "pane", id: "group-b", sessionId: "c", sessionIds: ["c"] },
      },
      activePaneId: "group-a",
      activeId: "b",
    });
    const moved = moveWorkspacePaneSession(snapshot.root, "group-a", "group-b", "b")!;

    expect(findWorkspacePane(moved, "group-a")).toMatchObject({ sessionId: "a", sessionIds: ["a"] });
    expect(findWorkspacePane(moved, "group-b")).toMatchObject({ sessionId: "b", sessionIds: ["c", "b"] });
  });

  it("splits one view from a group into a directional sibling group", () => {
    const snapshot = sanitizeWorkspaceSnapshot({
      version: 3,
      root: { kind: "pane", id: "group-a", sessionId: "b", sessionIds: ["a", "b"] },
      activePaneId: "group-a",
      activeId: "b",
    });
    const split = splitWorkspacePaneSessionToGroup(
      snapshot.root,
      "group-a",
      "b",
      "vertical",
      "group-b",
      "split-root",
      "first",
    )!;

    expect(split).toMatchObject({
      kind: "split",
      id: "split-root",
      direction: "vertical",
      first: { kind: "pane", id: "group-b", sessionId: "b", sessionIds: ["b"] },
      second: { kind: "pane", id: "group-a", sessionId: "a", sessionIds: ["a"] },
    });
  });

  it("collapses an empty source group and deduplicates the target view", () => {
    const snapshot = sanitizeWorkspaceSnapshot({
      version: 3,
      root: {
        kind: "split",
        id: "root",
        direction: "vertical",
        ratio: 0.5,
        first: { kind: "pane", id: "group-a", sessionId: "a", sessionIds: ["a"] },
        second: { kind: "pane", id: "group-b", sessionId: "b", sessionIds: ["b", "a"] },
      },
      activePaneId: "group-a",
      activeId: "a",
    });
    const moved = moveWorkspacePaneSession(snapshot.root, "group-a", "group-b", "a")!;

    expect(moved).toMatchObject({ kind: "pane", id: "group-b", sessionId: "a", sessionIds: ["b", "a"] });
  });

  it("merges a complete group into another group with stable deduplication", () => {
    const snapshot = sanitizeWorkspaceSnapshot({
      version: 3,
      root: {
        kind: "split",
        id: "root",
        direction: "vertical",
        ratio: 0.5,
        first: { kind: "pane", id: "group-a", sessionId: "b", sessionIds: ["a", "b"] },
        second: { kind: "pane", id: "group-b", sessionId: "c", sessionIds: ["c", "a"] },
      },
      activePaneId: "group-a",
      activeId: "b",
    });
    const merged = mergeWorkspacePaneGroups(snapshot.root, "group-a", "group-b")!;

    expect(merged).toMatchObject({ kind: "pane", id: "group-b", sessionId: "b", sessionIds: ["c", "a", "b"] });
  });

  it("refuses to overflow a target group", () => {
    const fullTabs = Array.from({ length: MAX_WORKSPACE_GROUP_TABS }, (_, index) => `target-${index}`);
    const snapshot = sanitizeWorkspaceSnapshot({
      version: 3,
      root: {
        kind: "split",
        id: "root",
        direction: "vertical",
        ratio: 0.5,
        first: { kind: "pane", id: "group-a", sessionId: "source", sessionIds: ["source"] },
        second: { kind: "pane", id: "group-b", sessionId: fullTabs[0], sessionIds: fullTabs },
      },
      activePaneId: "group-a",
      activeId: "source",
    });

    expect(moveWorkspacePaneSession(snapshot.root, "group-a", "group-b", "source")).toBe(snapshot.root);
    expect(mergeWorkspacePaneGroups(snapshot.root, "group-a", "group-b")).toBe(snapshot.root);
  });

  it("resolves unique startup sessions from recursive leaves", () => {
    const workspace = sanitizeWorkspaceSnapshot({
      version: 1,
      layout: "horizontal",
      paneIds: ["a", "a", "b"],
      activeId: "a",
    });
    expect(resolveStartupSessionIds("last", [], workspace, ["a", "b", "c"])).toEqual(["a", "b"]);
    expect(resolveStartupSessionIds("specific", ["c", "missing", "c", "a"], workspace, ["a", "b", "c"])).toEqual(["c", "a"]);
    expect(resolveStartupSessionIds("none", ["a"], workspace, ["a"])).toEqual([]);
  });

  it("bounds oversized recursive snapshots to sixteen panes", () => {
    const root = buildFixtureTree(Array.from({ length: 20 }, (_, index) => `session-${index}`));
    const snapshot = sanitizeWorkspaceSnapshot({
      version: 2,
      root,
      activePaneId: "missing",
      activeId: "session-19",
    });
    const panes = workspacePaneLeaves(snapshot.root);

    expect(panes).toHaveLength(16);
    expect(new Set(panes.map((pane) => pane.id)).size).toBe(16);
    expect(panes.every((pane) => pane.sessionId.startsWith("session-"))).toBe(true);
    expect(findWorkspacePane(snapshot.root, snapshot.activePaneId)).toBeDefined();
  });

  it("rejects runtime splits beyond the persisted depth limit", () => {
    let root = sanitizeWorkspaceSnapshot({ version: 1, layout: "single", activeId: "session-0" }).root!;
    let activePaneId = workspacePaneLeaves(root)[0].id;
    for (let depth = 1; depth <= MAX_WORKSPACE_DEPTH; depth += 1) {
      const nextPaneId = `pane-${depth}`;
      root = splitWorkspacePane(root, activePaneId, "vertical", `session-${depth}`, nextPaneId, `split-${depth}`);
      activePaneId = nextPaneId;
    }

    expect(workspacePaneLeaves(root)).toHaveLength(MAX_WORKSPACE_DEPTH + 1);
    expect(splitWorkspacePane(root, activePaneId, "vertical", "too-deep", "pane-too-deep", "split-too-deep")).toBe(root);
  });

  it("finds directional neighbors across nested split geometry", () => {
    const snapshot = sanitizeWorkspaceSnapshot({
      version: 2,
      root: {
        kind: "split",
        id: "root",
        direction: "vertical",
        ratio: 0.5,
        first: { kind: "pane", id: "pane-a", sessionId: "a" },
        second: {
          kind: "split",
          id: "right",
          direction: "horizontal",
          ratio: 0.4,
          first: { kind: "pane", id: "pane-b", sessionId: "b" },
          second: { kind: "pane", id: "pane-c", sessionId: "c" },
        },
      },
      activePaneId: "pane-a",
      activeId: "a",
    });

    expect(findWorkspacePaneInDirection(snapshot.root, "pane-a", "right")?.id).toBe("pane-c");
    expect(findWorkspacePaneInDirection(snapshot.root, "pane-b", "down")?.id).toBe("pane-c");
    expect(findWorkspacePaneInDirection(snapshot.root, "pane-c", "up")?.id).toBe("pane-b");
    expect(findWorkspacePaneInDirection(snapshot.root, "pane-b", "left")?.id).toBe("pane-a");
    expect(findWorkspacePaneInDirection(snapshot.root, "pane-b", "right")).toBeUndefined();
  });

  it("can place a new pane before the active pane", () => {
    const initial = sanitizeWorkspaceSnapshot({ version: 1, layout: "single", activeId: "a" }).root!;
    const activePaneId = workspacePaneLeaves(initial)[0].id;
    const split = splitWorkspacePane(initial, activePaneId, "vertical", "b", "pane-b", "split-left", "first");

    expect(workspacePaneLeaves(split).map((pane) => pane.sessionId)).toEqual(["b", "a"]);
  });

  it("swaps complete pane nodes while preserving the active pane identity", () => {
    const snapshot = sanitizeWorkspaceSnapshot({
      version: 1,
      layout: "horizontal",
      paneIds: ["a", "b", "c"],
      activeId: "b",
    });
    const panes = workspacePaneLeaves(snapshot.root);
    const swapped = swapWorkspacePanes(snapshot.root, panes[0].id, panes[1].id);

    expect(workspacePaneLeaves(swapped).map((pane) => pane.sessionId)).toEqual(["b", "a", "c"]);
    expect(findWorkspacePane(swapped, snapshot.activePaneId)?.sessionId).toBe("b");
    expect(swapWorkspacePanes(swapped, "missing", panes[2].id)).toBe(swapped);
  });

  it("sanitizes active identifiers when a v2 root is unavailable", () => {
    const snapshot = sanitizeWorkspaceSnapshot({
      version: 2,
      root: null,
      activePaneId: ` pane\n${"x".repeat(200)}`,
      activeId: ` session\n${"y".repeat(300)}`,
    });

    expect(snapshot.activeId).toHaveLength(256);
    expect(snapshot.activeId).not.toMatch(/[\r\n]/);
    expect(snapshot.activePaneId).toBe("pane-active");
  });
});

function findSplit(root: WorkspaceNode | null, id: string): WorkspaceSplitNode | undefined {
  if (!root || root.kind === "pane") return undefined;
  if (root.id === id) return root;
  return findSplit(root.first, id) ?? findSplit(root.second, id);
}

function buildFixtureTree(sessionIds: string[], depth = 0): unknown {
  if (sessionIds.length === 1) {
    return { kind: "pane", id: `pane-${sessionIds[0]}`, sessionId: sessionIds[0] };
  }
  const midpoint = Math.ceil(sessionIds.length / 2);
  return {
    kind: "split",
    id: `split-${depth}-${sessionIds.length}-${sessionIds[0]}`,
    direction: depth % 2 ? "horizontal" : "vertical",
    ratio: 0.5,
    first: buildFixtureTree(sessionIds.slice(0, midpoint), depth + 1),
    second: buildFixtureTree(sessionIds.slice(midpoint), depth + 1),
  };
}
