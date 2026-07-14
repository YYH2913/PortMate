import { describe, expect, it } from "vitest";
import {
  MAX_WORKSPACE_DEPTH,
  findWorkspacePane,
  reconcileWorkspaceSnapshot,
  removeWorkspacePane,
  replaceWorkspacePaneSession,
  resolveStartupSessionIds,
  sanitizeWorkspaceSnapshot,
  splitWorkspacePane,
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

    expect(snapshot.version).toBe(2);
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
