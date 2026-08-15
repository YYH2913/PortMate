import {
  removeWorkspacePaneView,
  workspacePaneActiveView,
  workspacePaneLeaves,
} from "./workspace-state";
import type { WorkspaceNode } from "./workspace-state";

export type WorkspaceViewDetachCommit = {
  status: "detached";
  root: WorkspaceNode;
  activePaneId: string;
  activeId: string;
} | {
  status: "missing" | "last-view";
};

export function commitWorkspaceViewDetach(
  root: WorkspaceNode | null,
  activePaneId: string,
  viewId: string,
  sessionId: string,
): WorkspaceViewDetachCommit {
  const panes = workspacePaneLeaves(root);
  const sourceIndex = panes.findIndex((pane) => pane.views.some((view) => (
    view.id === viewId && view.sessionId === sessionId
  )));
  if (sourceIndex < 0) return { status: "missing" };
  const totalViewCount = panes.reduce((count, pane) => count + pane.views.length, 0);
  if (totalViewCount <= 1) return { status: "last-view" };

  const source = panes[sourceIndex];
  const nextRoot = removeWorkspacePaneView(root, source.id, viewId);
  const nextPanes = workspacePaneLeaves(nextRoot);
  const nextActive = nextPanes.find((pane) => pane.id === activePaneId)
    ?? nextPanes[Math.min(sourceIndex, nextPanes.length - 1)];
  if (!nextRoot || !nextActive) return { status: "last-view" };
  return {
    status: "detached",
    root: nextRoot,
    activePaneId: nextActive.id,
    activeId: workspacePaneActiveView(nextActive).sessionId,
  };
}
