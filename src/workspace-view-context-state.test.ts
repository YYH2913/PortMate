import { describe, expect, it } from "vitest";
import { MAX_WORKSPACE_GROUP_TABS } from "./workspace-state";
import type { WorkspacePaneNode } from "./workspace-state";
import { workspaceSplitDirectionForVisualOrientation, workspaceViewContextCapabilities } from "./workspace-view-context-state";

function pane(viewCount = 3): WorkspacePaneNode {
  const views = Array.from({ length: viewCount }, (_, index) => ({
    id: `view-${index}`,
    sessionId: `session-${index}`,
    title: "",
    color: "",
    keyMode: "remote" as const,
  }));
  return {
    kind: "pane",
    id: "pane-a",
    activeViewId: views[0]?.id ?? "",
    views,
    sessionId: views[0]?.sessionId ?? "",
    sessionIds: views.map((view) => view.sessionId),
  };
}

describe("workspace view context capabilities", () => {
  it("maps visual split labels to the recursive tree axis", () => {
    expect(workspaceSplitDirectionForVisualOrientation("horizontal")).toBe("vertical");
    expect(workspaceSplitDirectionForVisualOrientation("vertical")).toBe("horizontal");
  });

  it("derives actions from the exact view index and workspace shape", () => {
    expect(workspaceViewContextCapabilities(pane(), "view-1", 1, 3, false)).toEqual({
      canDuplicate: true,
      canClose: true,
      canCloseOther: true,
      canCloseRight: true,
      canMove: false,
      canMoveToNewGroup: true,
      canDetach: true,
      canClosePane: false,
      canReopen: false,
    });
    expect(workspaceViewContextCapabilities(pane(), "view-2", 2, 3, true)).toMatchObject({
      canCloseRight: false,
      canMove: true,
      canMoveToNewGroup: true,
      canDetach: true,
      canClosePane: true,
      canReopen: true,
    });
  });

  it("protects the final view and disables actions for an unknown view", () => {
    expect(workspaceViewContextCapabilities(pane(1), "view-0", 1, 1, false)).toMatchObject({
      canClose: false,
      canCloseOther: false,
      canCloseRight: false,
      canMove: false,
      canMoveToNewGroup: false,
      canDetach: false,
      canClosePane: false,
    });
    expect(workspaceViewContextCapabilities(pane(), "missing", 2, 3, true))
      .toEqual({ canDuplicate: false, canClose: false, canCloseOther: false, canCloseRight: false, canMove: false, canMoveToNewGroup: false, canDetach: false, canClosePane: false, canReopen: false });
  });

  it("disables duplication at the per-group view limit", () => {
    expect(workspaceViewContextCapabilities(pane(MAX_WORKSPACE_GROUP_TABS), "view-0", 1, MAX_WORKSPACE_GROUP_TABS, false).canDuplicate)
      .toBe(false);
  });

  it("disables creating a group at the workspace structure limit", () => {
    expect(workspaceViewContextCapabilities(pane(), "view-0", 2, 3, false, false).canMoveToNewGroup).toBe(false);
  });
});
