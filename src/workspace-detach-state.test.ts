import { describe, expect, it } from "vitest";
import { commitWorkspaceViewDetach, commitWorkspaceViewReattach } from "./workspace-detach-state";
import { findWorkspacePane, workspacePaneLeaves } from "./workspace-state";
import type { WorkspaceNode } from "./workspace-state";

describe("workspace view detach commit", () => {
  it("removes the requested view from the latest layout without discarding newer tabs", () => {
    const root = pane("pane-a", "view-edge", [
      view("view-edge", "edge-router"),
      view("view-bench", "bench-uart"),
      view("view-local", "local-shell"),
    ]);

    const result = commitWorkspaceViewDetach(root, "pane-a", "view-edge", "edge-router");

    expect(result.status).toBe("detached");
    if (result.status !== "detached") return;
    expect(findWorkspacePane(result.root, "pane-a")?.views.map((item) => item.id))
      .toEqual(["view-bench", "view-local"]);
    expect(result.activeId).toBe("bench-uart");
  });

  it("finds a requested view that moved to another group while its window was opening", () => {
    const root: WorkspaceNode = {
      kind: "split",
      id: "split-root",
      direction: "vertical",
      ratio: 0.5,
      first: pane("pane-a", "view-local", [view("view-local", "local-shell")]),
      second: pane("pane-b", "view-edge", [
        view("view-bench", "bench-uart"),
        view("view-edge", "edge-router"),
      ]),
    };

    const result = commitWorkspaceViewDetach(root, "pane-a", "view-edge", "edge-router");

    expect(result.status).toBe("detached");
    if (result.status !== "detached") return;
    expect(workspacePaneLeaves(result.root).map((item) => item.views.map((candidate) => candidate.id)))
      .toEqual([["view-local"], ["view-bench"]]);
    expect(result.activePaneId).toBe("pane-a");
    expect(result.activeId).toBe("local-shell");
  });

  it("refuses to remove the final main-window view", () => {
    const root = pane("pane-a", "view-edge", [view("view-edge", "edge-router")]);

    expect(commitWorkspaceViewDetach(root, "pane-a", "view-edge", "edge-router"))
      .toEqual({ status: "last-view" });
  });

  it("treats an already removed or mismatched view as missing", () => {
    const root = pane("pane-a", "view-local", [view("view-local", "local-shell")]);

    expect(commitWorkspaceViewDetach(root, "pane-a", "view-edge", "edge-router"))
      .toEqual({ status: "missing" });
    expect(commitWorkspaceViewDetach(root, "pane-a", "view-local", "edge-router"))
      .toEqual({ status: "missing" });
  });
});

describe("workspace view reattach commit", () => {
  it("combines sequential returns against the latest layout", () => {
    const initial = pane("pane-main", "view-bench", [view("view-bench", "bench-uart")]);
    const first = commitWorkspaceViewReattach(
      initial,
      "pane-main",
      "pane-edge",
      view("view-edge", "edge-router"),
      () => "split-edge",
    );
    expect(first.status).toBe("reattached");
    if (first.status !== "reattached") return;

    const second = commitWorkspaceViewReattach(
      first.root,
      first.activePaneId,
      "pane-local",
      view("view-local", "local-shell"),
      () => "split-local",
    );
    expect(second.status).toBe("reattached");
    if (second.status !== "reattached") return;
    expect(workspacePaneLeaves(second.root).map((item) => item.views.map((candidate) => candidate.id)))
      .toEqual([["view-bench"], ["view-edge"], ["view-local"]]);
    expect(second.activePaneId).toBe("pane-local");
  });

  it("returns to an existing original group without discarding newer tabs", () => {
    const root = pane("pane-a", "view-local", [
      view("view-bench", "bench-uart"),
      view("view-local", "local-shell"),
    ]);

    const result = commitWorkspaceViewReattach(
      root,
      "pane-a",
      "pane-a",
      view("view-edge", "edge-router"),
    );

    expect(result.status).toBe("reattached");
    if (result.status !== "reattached") return;
    expect(result.placement).toBe("original-pane");
    expect(findWorkspacePane(result.root, "pane-a")?.views.map((item) => item.id))
      .toEqual(["view-bench", "view-local", "view-edge"]);
  });

  it("makes a repeated return idempotent", () => {
    const root = pane("pane-a", "view-bench", [
      view("view-bench", "bench-uart"),
      view("view-edge", "edge-router"),
    ]);

    const result = commitWorkspaceViewReattach(
      root,
      "pane-a",
      "pane-old",
      view("view-edge", "edge-router"),
    );

    expect(result.status).toBe("reattached");
    if (result.status !== "reattached") return;
    expect(result.placement).toBe("existing-view");
    expect(findWorkspacePane(result.root, "pane-a")?.views).toHaveLength(2);
    expect(findWorkspacePane(result.root, "pane-a")?.activeViewId).toBe("view-edge");
  });

  it("rejects a returned view ID already owned by another session", () => {
    const root = pane("pane-a", "view-edge", [view("view-edge", "edge-router")]);

    expect(commitWorkspaceViewReattach(
      root,
      "pane-a",
      "pane-old",
      view("view-edge", "local-shell"),
    )).toEqual({ status: "conflict" });
  });
});

function view(id: string, sessionId: string) {
  return { id, sessionId, title: "", color: "", keyMode: "remote" as const };
}

function pane(id: string, activeViewId: string, views: ReturnType<typeof view>[]) {
  const activeView = views.find((item) => item.id === activeViewId) ?? views[0];
  return {
    kind: "pane" as const,
    id,
    activeViewId: activeView.id,
    views,
    sessionId: activeView.sessionId,
    sessionIds: views.map((item) => item.sessionId),
  };
}
