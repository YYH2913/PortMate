import {
  activateWorkspacePaneView,
  createWorkspaceNodeId,
  createWorkspacePaneFromViews,
  findWorkspacePane,
  insertWorkspacePaneView,
  MAX_WORKSPACE_GROUP_TABS,
  MAX_WORKSPACE_PANES,
  removeWorkspacePaneView,
  replaceWorkspacePaneView,
  splitWorkspacePaneWithView,
  workspacePaneActiveView,
  workspacePaneLeaves,
} from "./workspace-state";
import type { WorkspaceNode, WorkspaceView } from "./workspace-state";

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

export type WorkspaceViewReattachPlacement =
  | "existing-view"
  | "original-pane"
  | "empty-workspace"
  | "new-pane"
  | "max-panes"
  | "max-depth";

export type WorkspaceViewReattachedCommit = {
  status: "reattached";
  root: WorkspaceNode;
  activePaneId: string;
  activeId: string;
  placement: WorkspaceViewReattachPlacement;
  replaced?: { view: WorkspaceView; paneId: string; index: number };
};

export type WorkspaceViewReattachCommit = WorkspaceViewReattachedCommit | {
  status: "conflict";
};

export function commitWorkspaceViewReattach(
  root: WorkspaceNode | null,
  activePaneId: string,
  requestedPaneId: string,
  returnedView: WorkspaceView,
  createSplitId: () => string = () => createWorkspaceNodeId("split"),
): WorkspaceViewReattachCommit {
  const panes = workspacePaneLeaves(root);
  const existingViewPane = panes.find((pane) => pane.views.some((view) => view.id === returnedView.id));
  if (existingViewPane) {
    const existingView = existingViewPane.views.find((view) => view.id === returnedView.id)!;
    if (existingView.sessionId !== returnedView.sessionId) return { status: "conflict" };
    return reattached(
      activateWorkspacePaneView(root, existingViewPane.id, returnedView.id)!,
      existingViewPane.id,
      existingView.sessionId,
      "existing-view",
    );
  }

  const originalPane = findWorkspacePane(root, requestedPaneId);
  if (originalPane) {
    return addReturnedView(root!, originalPane.id, returnedView, "original-pane");
  }

  if (!root) {
    const nextRoot = createWorkspacePaneFromViews(requestedPaneId, [returnedView], returnedView.id)!;
    return reattached(nextRoot, nextRoot.id, returnedView.sessionId, "empty-workspace");
  }

  const target = findWorkspacePane(root, activePaneId) ?? panes[0];
  if (!target) return { status: "conflict" };
  if (panes.length >= MAX_WORKSPACE_PANES) {
    return addReturnedView(root, target.id, returnedView, "max-panes");
  }

  for (const candidate of [target, ...panes.filter((pane) => pane.id !== target.id)]) {
    const nextRoot = splitWorkspacePaneWithView(
      root,
      candidate.id,
      "vertical",
      returnedView,
      requestedPaneId,
      createSplitId(),
      "second",
    );
    if (nextRoot !== root) {
      return reattached(nextRoot, requestedPaneId, returnedView.sessionId, "new-pane");
    }
  }

  return addReturnedView(root, target.id, returnedView, "max-depth");
}

function addReturnedView(
  root: WorkspaceNode,
  paneId: string,
  returnedView: WorkspaceView,
  placement: Extract<WorkspaceViewReattachPlacement, "original-pane" | "max-panes" | "max-depth">,
): WorkspaceViewReattachCommit {
  const pane = findWorkspacePane(root, paneId);
  if (!pane) return { status: "conflict" };
  if (pane.views.length < MAX_WORKSPACE_GROUP_TABS) {
    const nextRoot = insertWorkspacePaneView(root, pane.id, returnedView, pane.views.length);
    return nextRoot === root
      ? { status: "conflict" }
      : reattached(nextRoot!, pane.id, returnedView.sessionId, placement);
  }

  const replacedView = workspacePaneActiveView(pane);
  const nextRoot = replaceWorkspacePaneView(root, pane.id, returnedView);
  return nextRoot === root
    ? { status: "conflict" }
    : {
      ...reattached(nextRoot!, pane.id, returnedView.sessionId, placement),
      replaced: {
        view: replacedView,
        paneId: pane.id,
        index: pane.views.findIndex((view) => view.id === replacedView.id),
      },
    };
}

function reattached(
  root: WorkspaceNode,
  activePaneId: string,
  activeId: string,
  placement: WorkspaceViewReattachPlacement,
): WorkspaceViewReattachedCommit {
  return { status: "reattached", root, activePaneId, activeId, placement };
}
