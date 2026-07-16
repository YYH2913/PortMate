import { MAX_WORKSPACE_GROUP_TABS } from "./workspace-state";
import type { WorkspacePaneNode, WorkspaceSplitDirection } from "./workspace-state";

export type WorkspaceViewContextCapabilities = {
  canDuplicate: boolean;
  canClose: boolean;
  canCloseOther: boolean;
  canCloseRight: boolean;
  canMove: boolean;
  canReopen: boolean;
};

export function workspaceSplitDirectionForVisualOrientation(
  orientation: "horizontal" | "vertical",
): WorkspaceSplitDirection {
  return orientation === "horizontal" ? "vertical" : "horizontal";
}

export function workspaceViewContextCapabilities(
  pane: WorkspacePaneNode | undefined,
  viewId: string | undefined,
  paneCount: number,
  totalViewCount: number,
  canReopen: boolean,
): WorkspaceViewContextCapabilities {
  const viewIndex = pane?.views.findIndex((view) => view.id === viewId) ?? -1;
  const hasView = viewIndex >= 0;
  const viewCount = pane?.views.length ?? 0;
  return {
    canDuplicate: hasView && viewCount < MAX_WORKSPACE_GROUP_TABS,
    canClose: hasView && totalViewCount > 1,
    canCloseOther: hasView && viewCount > 1,
    canCloseRight: hasView && viewIndex < viewCount - 1,
    canMove: hasView && paneCount > 1,
    canReopen: hasView && canReopen,
  };
}
