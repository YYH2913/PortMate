export type WorkspaceLayout = "single" | "horizontal" | "vertical";
export type WorkspaceSplitDirection = Exclude<WorkspaceLayout, "single">;
export type WorkspacePaneDirection = "up" | "down" | "left" | "right";
export type WorkspaceSplitPlacement = "first" | "second";

export interface WorkspaceView {
  id: string;
  sessionId: string;
  title: string;
}

export interface WorkspacePaneNode {
  kind: "pane";
  id: string;
  activeViewId: string;
  views: WorkspaceView[];
  // Compatibility projections for session-oriented callers.
  sessionId: string;
  sessionIds: string[];
}

export interface WorkspaceSplitNode {
  kind: "split";
  id: string;
  direction: WorkspaceSplitDirection;
  ratio: number;
  first: WorkspaceNode;
  second: WorkspaceNode;
}

export type WorkspaceNode = WorkspacePaneNode | WorkspaceSplitNode;

export interface WorkspaceSnapshot {
  version: 4;
  root: WorkspaceNode | null;
  activePaneId: string;
  activeId: string;
  tabColors: Record<string, string>;
}

export type StartupMode = "none" | "last" | "specific";

export const MAX_WORKSPACE_PANES = 16;
export const MAX_WORKSPACE_DEPTH = 8;
export const MAX_WORKSPACE_GROUP_TABS = 32;
export const MIN_WORKSPACE_SPLIT_RATIO = 0.15;
export const MAX_WORKSPACE_SPLIT_RATIO = 0.85;

export const emptyWorkspaceSnapshot: WorkspaceSnapshot = {
  version: 4,
  root: null,
  activePaneId: "",
  activeId: "",
  tabColors: {},
};

let workspaceIdCounter = 0;

export function createWorkspaceNodeId(kind: "pane" | "split" | "view") {
  workspaceIdCounter += 1;
  const uuid = globalThis.crypto?.randomUUID?.();
  return uuid
    ? `${kind}-${uuid}`
    : `${kind}-${Date.now().toString(36)}-${workspaceIdCounter.toString(36)}`;
}

export function createWorkspaceView(
  sessionId: string,
  id = createWorkspaceNodeId("view"),
  title = "",
): WorkspaceView {
  return { id, sessionId, title };
}

export function createWorkspacePane(
  sessionId: string,
  id = createWorkspaceNodeId("pane"),
  sessionIds: string[] = [sessionId],
): WorkspacePaneNode {
  const requestedIds = uniqueStrings(sessionIds.includes(sessionId) ? sessionIds : [sessionId, ...sessionIds]);
  const normalizedIds = requestedIds.slice(0, MAX_WORKSPACE_GROUP_TABS);
  if (!normalizedIds.includes(sessionId)) normalizedIds[normalizedIds.length - 1] = sessionId;
  const views = normalizedIds.map((id) => createWorkspaceView(id));
  const activeViewId = views.find((view) => view.sessionId === sessionId)?.id ?? views[0]?.id ?? "";
  return createWorkspacePaneFromViews(id, views, activeViewId)!;
}

export function createWorkspacePaneFromViews(
  id: string,
  views: WorkspaceView[],
  activeViewId: string,
): WorkspacePaneNode | null {
  const normalizedViews = views.slice(0, MAX_WORKSPACE_GROUP_TABS).map((view) => ({ ...view }));
  if (!normalizedViews.length) return null;
  const activeView = normalizedViews.find((view) => view.id === activeViewId) ?? normalizedViews[0];
  return {
    kind: "pane",
    id,
    activeViewId: activeView.id,
    views: normalizedViews,
    sessionId: activeView.sessionId,
    sessionIds: normalizedViews.map((view) => view.sessionId),
  };
}

export function sanitizeWorkspaceSnapshot(value: unknown): WorkspaceSnapshot {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return { ...emptyWorkspaceSnapshot };
  }
  const source = value as Record<string, unknown>;
  const tabColors = sanitizeTabColors(source.tabColors);
  const requestedActiveId = cleanString(source.activeId, 256);
  const state: SanitizeState = { ids: new Set(), viewIds: new Set(), paneCount: 0 };
  const root = source.version === 2 || source.version === 3 || source.version === 4 || "root" in source
    ? sanitizeWorkspaceNode(source.root, "root", 0, state)
    : migrateLegacyWorkspace(source, state);
  const withSinglePane = root ?? (
    requestedActiveId ? createSanitizedPane(requestedActiveId, "pane-active", state) : null
  );
  const panes = workspacePaneLeaves(withSinglePane);
  const requestedActivePaneId = cleanString(source.activePaneId, 128);
  const activePane = panes.find((pane) => pane.id === requestedActivePaneId)
    ?? panes.find((pane) => pane.views.some((view) => view.sessionId === requestedActiveId))
    ?? panes[0];
  const activeSessionId = activePane?.sessionIds.includes(requestedActiveId)
    ? requestedActiveId
    : activePane?.sessionId ?? requestedActiveId;
  const activatedRoot = activePane && activeSessionId
    ? activateWorkspacePaneSession(withSinglePane, activePane.id, activeSessionId)
    : withSinglePane;
  return {
    version: 4,
    root: activatedRoot,
    activePaneId: activePane?.id ?? "",
    activeId: activeSessionId,
    tabColors,
  };
}

export function reconcileWorkspaceSnapshot(snapshot: WorkspaceSnapshot, sessionIds: string[]): WorkspaceSnapshot {
  const sanitized = sanitizeWorkspaceSnapshot(snapshot);
  const available = new Set(sessionIds);
  let root = reconcileWorkspaceNode(sanitized.root, available);
  if (!root) {
    const fallbackId = available.has(sanitized.activeId) ? sanitized.activeId : sessionIds[0] ?? "";
    root = fallbackId ? createWorkspacePane(fallbackId, "pane-default") : null;
  }
  const panes = workspacePaneLeaves(root);
  const activePane = panes.find((pane) => pane.id === sanitized.activePaneId)
    ?? panes.find((pane) => pane.sessionId === sanitized.activeId)
    ?? panes[0];
  const viewIds = new Set(panes.flatMap((pane) => pane.views.map((view) => view.id)));
  const tabColors = Object.fromEntries(Object.entries(sanitized.tabColors).filter(([id]) => (
    available.has(id) || viewIds.has(id)
  )));
  return {
    version: 4,
    root,
    activePaneId: activePane?.id ?? "",
    activeId: activePane?.sessionId ?? "",
    tabColors,
  };
}

export function resolveStartupSessionIds(
  mode: StartupMode,
  configuredIds: string[],
  workspace: WorkspaceSnapshot,
  sessionIds: string[],
): string[] {
  if (mode === "none") return [];
  const available = new Set(sessionIds);
  const requested = mode === "specific"
    ? configuredIds
    : workspacePaneLeaves(workspace.root).flatMap((pane) => pane.views.map((view) => view.sessionId));
  const fallback = requested.length ? requested : [workspace.activeId];
  return fallback.filter((id, index) => id && available.has(id) && fallback.indexOf(id) === index);
}

export function workspacePaneLeaves(root: WorkspaceNode | null): WorkspacePaneNode[] {
  if (!root) return [];
  if (root.kind === "pane") return [root];
  return [...workspacePaneLeaves(root.first), ...workspacePaneLeaves(root.second)];
}

export function findWorkspacePane(root: WorkspaceNode | null, paneId: string): WorkspacePaneNode | undefined {
  return workspacePaneLeaves(root).find((pane) => pane.id === paneId);
}

export function findWorkspacePaneBySession(root: WorkspaceNode | null, sessionId: string): WorkspacePaneNode | undefined {
  return workspacePaneLeaves(root).find((pane) => pane.views.some((view) => view.sessionId === sessionId));
}

export function workspacePaneActiveView(pane: WorkspacePaneNode): WorkspaceView {
  return pane.views.find((view) => view.id === pane.activeViewId) ?? pane.views[0];
}

export function findWorkspaceView(
  root: WorkspaceNode | null,
  paneId: string,
  viewId: string,
): WorkspaceView | undefined {
  return findWorkspacePane(root, paneId)?.views.find((view) => view.id === viewId);
}

export function findWorkspacePaneInDirection(
  root: WorkspaceNode | null,
  paneId: string,
  direction: WorkspacePaneDirection,
): WorkspacePaneNode | undefined {
  const rectangles = workspacePaneRectangles(root);
  const active = rectangles.find((item) => item.pane.id === paneId);
  if (!active) return undefined;
  return rectangles
    .map((candidate, index) => ({ candidate, index, metrics: directionalPaneMetrics(active, candidate, direction) }))
    .filter((item): item is { candidate: WorkspacePaneRectangle; index: number; metrics: [number, number, number] } => Boolean(item.metrics))
    .sort((left, right) => (
      left.metrics[0] - right.metrics[0]
      || left.metrics[1] - right.metrics[1]
      || left.metrics[2] - right.metrics[2]
      || left.index - right.index
    ))[0]?.candidate.pane;
}

export function replaceWorkspacePaneSession(
  root: WorkspaceNode | null,
  paneId: string,
  sessionId: string,
): WorkspaceNode | null {
  if (!root) return root;
  if (root.kind === "pane") {
    if (root.id !== paneId) return root;
    const existing = root.views.find((view) => view.sessionId === sessionId);
    if (existing) {
      return root.activeViewId === existing.id ? root : createWorkspacePaneFromViews(root.id, root.views, existing.id)!;
    }
    const activeIndex = root.views.findIndex((view) => view.id === root.activeViewId);
    const views = [...root.views];
    const replacement = createWorkspaceView(sessionId, activeIndex >= 0 ? views[activeIndex].id : undefined);
    if (activeIndex >= 0) views.splice(activeIndex, 1, replacement);
    else views.push(replacement);
    return createWorkspacePaneFromViews(root.id, views, replacement.id)!;
  }
  const first = replaceWorkspacePaneSession(root.first, paneId, sessionId);
  const second = replaceWorkspacePaneSession(root.second, paneId, sessionId);
  return first === root.first && second === root.second ? root : { ...root, first: first!, second: second! };
}

export function addWorkspacePaneSession(
  root: WorkspaceNode | null,
  paneId: string,
  sessionId: string,
): WorkspaceNode | null {
  return insertWorkspacePaneSession(root, paneId, sessionId, Number.POSITIVE_INFINITY);
}

export function insertWorkspacePaneSession(
  root: WorkspaceNode | null,
  paneId: string,
  sessionId: string,
  index: number,
): WorkspaceNode | null {
  if (!root) return root;
  if (root.kind === "pane") {
    if (root.id !== paneId) return root;
    const existing = root.views.find((view) => view.sessionId === sessionId);
    if (existing) {
      return root.activeViewId === existing.id ? root : createWorkspacePaneFromViews(root.id, root.views, existing.id)!;
    }
    return insertWorkspacePaneView(root, paneId, createWorkspaceView(sessionId), index);
  }
  const first = insertWorkspacePaneSession(root.first, paneId, sessionId, index);
  const second = insertWorkspacePaneSession(root.second, paneId, sessionId, index);
  return first === root.first && second === root.second ? root : { ...root, first: first!, second: second! };
}

export function activateWorkspacePaneSession(
  root: WorkspaceNode | null,
  paneId: string,
  sessionId: string,
): WorkspaceNode | null {
  if (!root) return root;
  if (root.kind === "pane") {
    if (root.id !== paneId || root.sessionId === sessionId) return root;
    const view = root.views.find((candidate) => candidate.sessionId === sessionId);
    return view ? createWorkspacePaneFromViews(root.id, root.views, view.id)! : root;
  }
  const first = activateWorkspacePaneSession(root.first, paneId, sessionId);
  const second = activateWorkspacePaneSession(root.second, paneId, sessionId);
  return first === root.first && second === root.second ? root : { ...root, first: first!, second: second! };
}

export function activateWorkspacePaneView(
  root: WorkspaceNode | null,
  paneId: string,
  viewId: string,
): WorkspaceNode | null {
  if (!root) return root;
  if (root.kind === "pane") {
    if (root.id !== paneId || root.activeViewId === viewId || !root.views.some((view) => view.id === viewId)) return root;
    return createWorkspacePaneFromViews(root.id, root.views, viewId)!;
  }
  const first = activateWorkspacePaneView(root.first, paneId, viewId);
  const second = activateWorkspacePaneView(root.second, paneId, viewId);
  return first === root.first && second === root.second ? root : { ...root, first: first!, second: second! };
}

export function insertWorkspacePaneView(
  root: WorkspaceNode | null,
  paneId: string,
  view: WorkspaceView,
  index: number,
): WorkspaceNode | null {
  if (!root) return root;
  if (root.kind === "pane") {
    if (root.id !== paneId || root.views.length >= MAX_WORKSPACE_GROUP_TABS) return root;
    if (workspacePaneLeaves(root).some((pane) => pane.views.some((item) => item.id === view.id))) return root;
    const requestedIndex = Number.isFinite(index) ? Math.trunc(index) : root.views.length;
    const insertionIndex = Math.min(root.views.length, Math.max(0, requestedIndex));
    const views = [...root.views];
    views.splice(insertionIndex, 0, { ...view });
    return createWorkspacePaneFromViews(root.id, views, view.id)!;
  }
  if (workspacePaneLeaves(root).some((pane) => pane.views.some((item) => item.id === view.id))) return root;
  const first = insertWorkspacePaneView(root.first, paneId, view, index);
  const second = first === root.first ? insertWorkspacePaneView(root.second, paneId, view, index) : root.second;
  return first === root.first && second === root.second ? root : { ...root, first: first!, second: second! };
}

export function duplicateWorkspacePaneView(
  root: WorkspaceNode | null,
  paneId: string,
  viewId: string,
  duplicateId = createWorkspaceNodeId("view"),
  title?: string,
): WorkspaceNode | null {
  const pane = findWorkspacePane(root, paneId);
  const source = pane?.views.find((view) => view.id === viewId);
  if (!pane || !source || pane.views.length >= MAX_WORKSPACE_GROUP_TABS) return root;
  const index = pane.views.findIndex((view) => view.id === viewId);
  return insertWorkspacePaneView(root, paneId, {
    id: duplicateId,
    sessionId: source.sessionId,
    title: title ?? source.title,
  }, index + 1);
}

export function renameWorkspacePaneView(
  root: WorkspaceNode | null,
  paneId: string,
  viewId: string,
  title: string,
): WorkspaceNode | null {
  if (!root) return root;
  if (root.kind === "pane") {
    if (root.id !== paneId) return root;
    const index = root.views.findIndex((view) => view.id === viewId);
    const cleanTitle = cleanString(title, 128);
    if (index < 0 || root.views[index].title === cleanTitle) return root;
    const views = [...root.views];
    views[index] = { ...views[index], title: cleanTitle };
    return createWorkspacePaneFromViews(root.id, views, root.activeViewId)!;
  }
  const first = renameWorkspacePaneView(root.first, paneId, viewId, title);
  const second = renameWorkspacePaneView(root.second, paneId, viewId, title);
  return first === root.first && second === root.second ? root : { ...root, first: first!, second: second! };
}

export function replaceWorkspacePaneView(
  root: WorkspaceNode | null,
  paneId: string,
  view: WorkspaceView,
): WorkspaceNode | null {
  if (!root) return root;
  if (root.kind === "pane") {
    if (root.id !== paneId) return root;
    const activeIndex = root.views.findIndex((candidate) => candidate.id === root.activeViewId);
    if (activeIndex < 0 || root.views.some((candidate, index) => candidate.id === view.id && index !== activeIndex)) return root;
    const views = [...root.views];
    views[activeIndex] = { ...view };
    return createWorkspacePaneFromViews(root.id, views, view.id);
  }
  if (workspacePaneLeaves(root).some((pane) => pane.id !== paneId && pane.views.some((candidate) => candidate.id === view.id))) return root;
  const first = replaceWorkspacePaneView(root.first, paneId, view);
  const second = replaceWorkspacePaneView(root.second, paneId, view);
  return first === root.first && second === root.second ? root : { ...root, first: first!, second: second! };
}

export function moveWorkspacePaneSession(
  root: WorkspaceNode | null,
  sourcePaneId: string,
  targetPaneId: string,
  sessionId: string,
): WorkspaceNode | null {
  if (!root || sourcePaneId === targetPaneId) return root;
  const source = findWorkspacePane(root, sourcePaneId);
  const target = findWorkspacePane(root, targetPaneId);
  const sourceView = source?.views.find((view) => view.sessionId === sessionId);
  if (!source || !sourceView || !target) return root;
  const existingTarget = target.views.find((view) => view.sessionId === sessionId);
  if (existingTarget) {
    const removed = removeWorkspacePaneView(root, sourcePaneId, sourceView.id);
    return activateWorkspacePaneView(removed, targetPaneId, existingTarget.id);
  }
  return moveWorkspacePaneView(root, sourcePaneId, targetPaneId, sourceView.id);
}

export function moveWorkspacePaneView(
  root: WorkspaceNode | null,
  sourcePaneId: string,
  targetPaneId: string,
  viewId: string,
): WorkspaceNode | null {
  if (!root || sourcePaneId === targetPaneId) return root;
  const source = findWorkspacePane(root, sourcePaneId);
  const target = findWorkspacePane(root, targetPaneId);
  const view = source?.views.find((candidate) => candidate.id === viewId);
  if (!source || !target || !view || target.views.some((candidate) => candidate.id === viewId)) return root;
  if (target.views.length >= MAX_WORKSPACE_GROUP_TABS) return root;
  return moveWorkspacePaneViewInNode(root, sourcePaneId, targetPaneId, view);
}

export function removeWorkspacePaneSession(
  root: WorkspaceNode | null,
  paneId: string,
  sessionId: string,
): WorkspaceNode | null {
  if (!root) return root;
  if (root.kind === "pane") {
    if (root.id !== paneId) return root;
    const viewIds = root.views.filter((view) => view.sessionId === sessionId).map((view) => view.id);
    if (!viewIds.length) return root;
    let next: WorkspaceNode | null = root;
    for (const viewId of viewIds) next = removeWorkspacePaneView(next, paneId, viewId);
    return next;
  }
  const first = removeWorkspacePaneSession(root.first, paneId, sessionId);
  const second = removeWorkspacePaneSession(root.second, paneId, sessionId);
  if (!first) return second;
  if (!second) return first;
  return first === root.first && second === root.second ? root : { ...root, first, second };
}

export function removeWorkspacePaneView(
  root: WorkspaceNode | null,
  paneId: string,
  viewId: string,
): WorkspaceNode | null {
  if (!root) return root;
  if (root.kind === "pane") {
    if (root.id !== paneId) return root;
    const removedIndex = root.views.findIndex((view) => view.id === viewId);
    if (removedIndex < 0) return root;
    const views = root.views.filter((view) => view.id !== viewId);
    if (!views.length) return null;
    const activeViewId = root.activeViewId === viewId
      ? views[Math.min(removedIndex, views.length - 1)].id
      : root.activeViewId;
    return createWorkspacePaneFromViews(root.id, views, activeViewId)!;
  }
  const first = removeWorkspacePaneView(root.first, paneId, viewId);
  const second = removeWorkspacePaneView(root.second, paneId, viewId);
  if (!first) return second;
  if (!second) return first;
  return first === root.first && second === root.second ? root : { ...root, first, second };
}

export function splitWorkspacePaneSessionToGroup(
  root: WorkspaceNode | null,
  sourcePaneId: string,
  sessionId: string,
  direction: WorkspaceSplitDirection,
  newPaneId = createWorkspaceNodeId("pane"),
  splitId = createWorkspaceNodeId("split"),
  placement: WorkspaceSplitPlacement = "second",
): WorkspaceNode | null {
  const source = findWorkspacePane(root, sourcePaneId);
  const view = source?.views.find((candidate) => candidate.sessionId === sessionId);
  return view ? splitWorkspacePaneViewToGroup(
    root,
    sourcePaneId,
    view.id,
    direction,
    newPaneId,
    splitId,
    placement,
  ) : root;
}

export function splitWorkspacePaneViewToGroup(
  root: WorkspaceNode | null,
  sourcePaneId: string,
  viewId: string,
  direction: WorkspaceSplitDirection,
  newPaneId = createWorkspaceNodeId("pane"),
  splitId = createWorkspaceNodeId("split"),
  placement: WorkspaceSplitPlacement = "second",
): WorkspaceNode | null {
  if (!root || workspacePaneLeaves(root).length >= MAX_WORKSPACE_PANES) return root;
  const source = findWorkspacePane(root, sourcePaneId);
  const view = source?.views.find((candidate) => candidate.id === viewId);
  if (!source || !view || source.views.length <= 1) return root;
  const withoutView = removeWorkspacePaneView(root, sourcePaneId, viewId);
  if (!withoutView) return root;
  const splitRoot = splitWorkspacePaneWithView(
    withoutView,
    sourcePaneId,
    direction,
    view,
    newPaneId,
    splitId,
    placement,
  );
  return splitRoot === withoutView ? root : splitRoot;
}

export function mergeWorkspacePaneGroups(
  root: WorkspaceNode | null,
  sourcePaneId: string,
  targetPaneId: string,
): WorkspaceNode | null {
  if (!root || sourcePaneId === targetPaneId) return root;
  const source = findWorkspacePane(root, sourcePaneId);
  const target = findWorkspacePane(root, targetPaneId);
  if (!source || !target) return root;
  const mergedViews = [...target.views, ...source.views].map((view) => ({ ...view }));
  if (mergedViews.length > MAX_WORKSPACE_GROUP_TABS) return root;
  return mergeWorkspacePaneGroupsInNode(root, sourcePaneId, targetPaneId, source.activeViewId, mergedViews);
}

function mergeWorkspacePaneGroupsInNode(
  root: WorkspaceNode,
  sourcePaneId: string,
  targetPaneId: string,
  activeViewId: string,
  mergedViews: WorkspaceView[],
): WorkspaceNode | null {
  if (root.kind === "pane") {
    if (root.id === sourcePaneId) return null;
    if (root.id === targetPaneId) return createWorkspacePaneFromViews(root.id, mergedViews, activeViewId);
    return root;
  }
  const first = mergeWorkspacePaneGroupsInNode(root.first, sourcePaneId, targetPaneId, activeViewId, mergedViews);
  const second = mergeWorkspacePaneGroupsInNode(root.second, sourcePaneId, targetPaneId, activeViewId, mergedViews);
  if (!first) return second;
  if (!second) return first;
  return first === root.first && second === root.second ? root : { ...root, first, second };
}

function moveWorkspacePaneViewInNode(
  root: WorkspaceNode,
  sourcePaneId: string,
  targetPaneId: string,
  view: WorkspaceView,
): WorkspaceNode | null {
  if (root.kind === "pane") {
    if (root.id === targetPaneId) {
      return createWorkspacePaneFromViews(root.id, [...root.views, { ...view }], view.id);
    }
    if (root.id !== sourcePaneId) return root;
    const removedIndex = root.views.findIndex((candidate) => candidate.id === view.id);
    const views = root.views.filter((candidate) => candidate.id !== view.id);
    if (!views.length) return null;
    const activeViewId = root.activeViewId === view.id
      ? views[Math.min(removedIndex, views.length - 1)].id
      : root.activeViewId;
    return createWorkspacePaneFromViews(root.id, views, activeViewId);
  }
  const first = moveWorkspacePaneViewInNode(root.first, sourcePaneId, targetPaneId, view);
  const second = moveWorkspacePaneViewInNode(root.second, sourcePaneId, targetPaneId, view);
  if (!first) return second;
  if (!second) return first;
  return first === root.first && second === root.second ? root : { ...root, first, second };
}

export function swapWorkspacePanes(
  root: WorkspaceNode | null,
  firstPaneId: string,
  secondPaneId: string,
): WorkspaceNode | null {
  if (!root || firstPaneId === secondPaneId) return root;
  const firstPane = findWorkspacePane(root, firstPaneId);
  const secondPane = findWorkspacePane(root, secondPaneId);
  if (!firstPane || !secondPane) return root;
  return replaceWorkspacePaneNodes(root, firstPane, secondPane);
}

function replaceWorkspacePaneNodes(
  root: WorkspaceNode,
  firstPane: WorkspacePaneNode,
  secondPane: WorkspacePaneNode,
): WorkspaceNode {
  if (root.kind === "pane") {
    if (root.id === firstPane.id) return secondPane;
    if (root.id === secondPane.id) return firstPane;
    return root;
  }
  const first = replaceWorkspacePaneNodes(root.first, firstPane, secondPane);
  const second = replaceWorkspacePaneNodes(root.second, firstPane, secondPane);
  return first === root.first && second === root.second ? root : { ...root, first, second };
}

export function splitWorkspacePane(
  root: WorkspaceNode,
  paneId: string,
  direction: WorkspaceSplitDirection,
  newSessionId: string,
  newPaneId = createWorkspaceNodeId("pane"),
  splitId = createWorkspaceNodeId("split"),
  placement: WorkspaceSplitPlacement = "second",
): WorkspaceNode {
  return splitWorkspacePaneAtDepth(root, paneId, direction, newSessionId, newPaneId, splitId, placement, 0);
}

export function splitWorkspacePaneWithView(
  root: WorkspaceNode,
  paneId: string,
  direction: WorkspaceSplitDirection,
  view: WorkspaceView,
  newPaneId: string,
  splitId: string,
  placement: WorkspaceSplitPlacement,
): WorkspaceNode {
  return splitWorkspacePaneWithViewAtDepth(root, paneId, direction, view, newPaneId, splitId, placement, 0);
}

function splitWorkspacePaneWithViewAtDepth(
  root: WorkspaceNode,
  paneId: string,
  direction: WorkspaceSplitDirection,
  view: WorkspaceView,
  newPaneId: string,
  splitId: string,
  placement: WorkspaceSplitPlacement,
  depth: number,
): WorkspaceNode {
  if (root.kind === "pane") {
    if (root.id !== paneId || depth >= MAX_WORKSPACE_DEPTH) return root;
    const nextPane = createWorkspacePaneFromViews(newPaneId, [view], view.id)!;
    return {
      kind: "split",
      id: splitId,
      direction,
      ratio: 0.5,
      first: placement === "first" ? nextPane : root,
      second: placement === "first" ? root : nextPane,
    };
  }
  const first = splitWorkspacePaneWithViewAtDepth(root.first, paneId, direction, view, newPaneId, splitId, placement, depth + 1);
  if (first !== root.first) return { ...root, first };
  const second = splitWorkspacePaneWithViewAtDepth(root.second, paneId, direction, view, newPaneId, splitId, placement, depth + 1);
  return second === root.second ? root : { ...root, second };
}

function splitWorkspacePaneAtDepth(
  root: WorkspaceNode,
  paneId: string,
  direction: WorkspaceSplitDirection,
  newSessionId: string,
  newPaneId: string,
  splitId: string,
  placement: WorkspaceSplitPlacement,
  depth: number,
): WorkspaceNode {
  if (root.kind === "pane") {
    if (root.id !== paneId || depth >= MAX_WORKSPACE_DEPTH) return root;
    const nextPane = createWorkspacePane(newSessionId, newPaneId);
    return {
      kind: "split",
      id: splitId,
      direction,
      ratio: 0.5,
      first: placement === "first" ? nextPane : root,
      second: placement === "first" ? root : nextPane,
    };
  }
  const first = splitWorkspacePaneAtDepth(root.first, paneId, direction, newSessionId, newPaneId, splitId, placement, depth + 1);
  if (first !== root.first) return { ...root, first };
  const second = splitWorkspacePaneAtDepth(root.second, paneId, direction, newSessionId, newPaneId, splitId, placement, depth + 1);
  return second === root.second ? root : { ...root, second };
}

export function removeWorkspacePane(root: WorkspaceNode | null, paneId: string): WorkspaceNode | null {
  if (!root) return null;
  if (root.kind === "pane") return root.id === paneId ? null : root;
  const first = removeWorkspacePane(root.first, paneId);
  const second = removeWorkspacePane(root.second, paneId);
  if (!first) return second;
  if (!second) return first;
  return first === root.first && second === root.second ? root : { ...root, first, second };
}

export function updateWorkspaceSplitRatio(
  root: WorkspaceNode | null,
  splitId: string,
  ratio: number,
): WorkspaceNode | null {
  if (!root || root.kind === "pane") return root;
  if (root.id === splitId) return { ...root, ratio: normalizeSplitRatio(ratio) };
  const first = updateWorkspaceSplitRatio(root.first, splitId, ratio);
  const second = updateWorkspaceSplitRatio(root.second, splitId, ratio);
  return first === root.first && second === root.second ? root : { ...root, first: first!, second: second! };
}

type SanitizeState = {
  ids: Set<string>;
  viewIds: Set<string>;
  paneCount: number;
};

type WorkspacePaneRectangle = {
  pane: WorkspacePaneNode;
  left: number;
  top: number;
  width: number;
  height: number;
};

function workspacePaneRectangles(root: WorkspaceNode | null): WorkspacePaneRectangle[] {
  const rectangles: WorkspacePaneRectangle[] = [];
  collectWorkspacePaneRectangles(root, 0, 0, 1, 1, rectangles);
  return rectangles;
}

function collectWorkspacePaneRectangles(
  root: WorkspaceNode | null,
  left: number,
  top: number,
  width: number,
  height: number,
  rectangles: WorkspacePaneRectangle[],
) {
  if (!root) return;
  if (root.kind === "pane") {
    rectangles.push({ pane: root, left, top, width, height });
    return;
  }
  const ratio = normalizeSplitRatio(root.ratio);
  if (root.direction === "vertical") {
    const firstWidth = width * ratio;
    collectWorkspacePaneRectangles(root.first, left, top, firstWidth, height, rectangles);
    collectWorkspacePaneRectangles(root.second, left + firstWidth, top, width - firstWidth, height, rectangles);
    return;
  }
  const firstHeight = height * ratio;
  collectWorkspacePaneRectangles(root.first, left, top, width, firstHeight, rectangles);
  collectWorkspacePaneRectangles(root.second, left, top + firstHeight, width, height - firstHeight, rectangles);
}

function directionalPaneMetrics(
  active: WorkspacePaneRectangle,
  candidate: WorkspacePaneRectangle,
  direction: WorkspacePaneDirection,
): [number, number, number] | null {
  if (candidate.pane.id === active.pane.id) return null;
  const epsilon = 1e-9;
  const activeRight = active.left + active.width;
  const activeBottom = active.top + active.height;
  const candidateRight = candidate.left + candidate.width;
  const candidateBottom = candidate.top + candidate.height;
  const horizontal = direction === "left" || direction === "right";
  const overlap = horizontal
    ? Math.min(activeBottom, candidateBottom) - Math.max(active.top, candidate.top)
    : Math.min(activeRight, candidateRight) - Math.max(active.left, candidate.left);
  if (overlap <= epsilon) return null;

  let primaryGap: number;
  if (direction === "left") {
    if (candidateRight > active.left + epsilon) return null;
    primaryGap = active.left - candidateRight;
  } else if (direction === "right") {
    if (candidate.left < activeRight - epsilon) return null;
    primaryGap = candidate.left - activeRight;
  } else if (direction === "up") {
    if (candidateBottom > active.top + epsilon) return null;
    primaryGap = active.top - candidateBottom;
  } else {
    if (candidate.top < activeBottom - epsilon) return null;
    primaryGap = candidate.top - activeBottom;
  }
  const activeCrossCenter = horizontal ? active.top + active.height / 2 : active.left + active.width / 2;
  const candidateCrossCenter = horizontal ? candidate.top + candidate.height / 2 : candidate.left + candidate.width / 2;
  return [primaryGap, Math.abs(activeCrossCenter - candidateCrossCenter), -overlap];
}

function sanitizeWorkspaceNode(
  value: unknown,
  path: string,
  depth: number,
  state: SanitizeState,
): WorkspaceNode | null {
  if (!value || typeof value !== "object" || Array.isArray(value) || depth > MAX_WORKSPACE_DEPTH) {
    return null;
  }
  const source = value as Record<string, unknown>;
  if (source.kind === "pane") {
    const paneId = uniqueNodeId(cleanString(source.id, 128) || `pane-${path}`, state);
    const requestedActiveSessionId = cleanString(source.sessionId, 256);
    const storedViews = sanitizeWorkspaceViews(source.views, paneId, state);
    const legacySessionIds = uniqueStrings(validStrings(source.sessionIds));
    const requestedLegacyIds = requestedActiveSessionId && !legacySessionIds.includes(requestedActiveSessionId)
      ? [requestedActiveSessionId, ...legacySessionIds]
      : legacySessionIds;
    const views = storedViews.length
      ? storedViews
      : requestedLegacyIds.slice(0, MAX_WORKSPACE_GROUP_TABS).map((sessionId, index) => ({
        id: uniqueViewId(`view-${paneId}-${index + 1}`, state),
        sessionId,
        title: "",
      }));
    if (!views.length || state.paneCount >= MAX_WORKSPACE_PANES) return null;
    const requestedActiveViewId = cleanString(source.activeViewId, 128);
    const activeView = views.find((view) => view.id === requestedActiveViewId)
      ?? views.find((view) => view.sessionId === requestedActiveSessionId)
      ?? views[0];
    state.paneCount += 1;
    return createWorkspacePaneFromViews(paneId, views, activeView.id);
  }
  if (source.kind !== "split") return null;
  const first = sanitizeWorkspaceNode(source.first, `${path}-first`, depth + 1, state);
  const second = sanitizeWorkspaceNode(source.second, `${path}-second`, depth + 1, state);
  if (!first) return second;
  if (!second) return first;
  return {
    kind: "split",
    id: uniqueNodeId(cleanString(source.id, 128) || `split-${path}`, state),
    direction: isSplitDirection(source.direction) ? source.direction : "horizontal",
    ratio: normalizeSplitRatio(source.ratio),
    first,
    second,
  };
}

function migrateLegacyWorkspace(source: Record<string, unknown>, state: SanitizeState): WorkspaceNode | null {
  const layout = isWorkspaceLayout(source.layout) ? source.layout : "single";
  const paneIds = validStrings(source.paneIds).slice(0, 4);
  const activeId = cleanString(source.activeId, 256);
  if (layout === "single" || paneIds.length < 2) {
    return activeId ? createSanitizedPane(activeId, "pane-legacy-active", state) : null;
  }
  return buildLegacySplit(paneIds, layout, 0, state);
}

function buildLegacySplit(
  sessionIds: string[],
  direction: WorkspaceSplitDirection,
  offset: number,
  state: SanitizeState,
): WorkspaceNode {
  const first = createSanitizedPane(sessionIds[0], `pane-legacy-${offset}`, state)!;
  if (sessionIds.length === 1) return first;
  return {
    kind: "split",
    id: uniqueNodeId(`split-legacy-${offset}`, state),
    direction,
    ratio: 1 / sessionIds.length,
    first,
    second: buildLegacySplit(sessionIds.slice(1), direction, offset + 1, state),
  };
}

function createSanitizedPane(
  sessionId: string,
  id: string,
  state: SanitizeState,
  sessionIds: string[] = [sessionId],
): WorkspacePaneNode | null {
  if (state.paneCount >= MAX_WORKSPACE_PANES) return null;
  const paneId = uniqueNodeId(id, state);
  const views = uniqueStrings(sessionIds).slice(0, MAX_WORKSPACE_GROUP_TABS).map((candidate, index) => ({
    id: uniqueViewId(`view-${paneId}-${index + 1}`, state),
    sessionId: candidate,
    title: "",
  }));
  const activeView = views.find((view) => view.sessionId === sessionId) ?? views[0];
  if (!activeView) return null;
  state.paneCount += 1;
  return createWorkspacePaneFromViews(paneId, views, activeView.id);
}

function reconcileWorkspaceNode(root: WorkspaceNode | null, available: Set<string>): WorkspaceNode | null {
  if (!root) return null;
  if (root.kind === "pane") {
    const views = root.views.filter((view) => available.has(view.sessionId));
    if (!views.length) return null;
    const activeViewId = views.some((view) => view.id === root.activeViewId) ? root.activeViewId : views[0].id;
    return activeViewId === root.activeViewId && views.length === root.views.length
      ? root
      : createWorkspacePaneFromViews(root.id, views, activeViewId);
  }
  const first = reconcileWorkspaceNode(root.first, available);
  const second = reconcileWorkspaceNode(root.second, available);
  if (!first) return second;
  if (!second) return first;
  return first === root.first && second === root.second ? root : { ...root, first, second };
}

function uniqueNodeId(requested: string, state: SanitizeState) {
  let id = requested;
  let suffix = 2;
  while (state.ids.has(id)) {
    id = `${requested}-${suffix}`;
    suffix += 1;
  }
  state.ids.add(id);
  return id;
}

function uniqueViewId(requested: string, state: SanitizeState) {
  let id = requested;
  let suffix = 2;
  while (state.viewIds.has(id) || state.ids.has(id)) {
    id = `${requested}-${suffix}`;
    suffix += 1;
  }
  state.viewIds.add(id);
  return id;
}

function sanitizeWorkspaceViews(value: unknown, paneId: string, state: SanitizeState): WorkspaceView[] {
  if (!Array.isArray(value)) return [];
  const views: WorkspaceView[] = [];
  for (const [index, item] of value.entries()) {
    if (!item || typeof item !== "object" || Array.isArray(item)) continue;
    const source = item as Record<string, unknown>;
    const sessionId = cleanString(source.sessionId, 256);
    if (!sessionId) continue;
    const requestedId = cleanString(source.id, 128) || `view-${paneId}-${index + 1}`;
    views.push({
      id: uniqueViewId(requestedId, state),
      sessionId,
      title: cleanString(source.title, 128),
    });
    if (views.length >= MAX_WORKSPACE_GROUP_TABS) break;
  }
  return views;
}

function normalizeSplitRatio(value: unknown) {
  const ratio = typeof value === "number" && Number.isFinite(value) ? value : 0.5;
  return Math.min(MAX_WORKSPACE_SPLIT_RATIO, Math.max(MIN_WORKSPACE_SPLIT_RATIO, ratio));
}

function isWorkspaceLayout(value: unknown): value is WorkspaceLayout {
  return value === "single" || isSplitDirection(value);
}

function isSplitDirection(value: unknown): value is WorkspaceSplitDirection {
  return value === "horizontal" || value === "vertical";
}

function validStrings(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.map((item) => cleanString(item, 256)).filter(Boolean);
}

function uniqueStrings(values: string[]): string[] {
  return values.filter((value, index) => value && values.indexOf(value) === index);
}

function cleanString(value: unknown, maxLength: number): string {
  if (typeof value !== "string") return "";
  return [...value.trim()].filter((character) => !/\p{C}/u.test(character)).slice(0, maxLength).join("");
}

function sanitizeTabColors(value: unknown): Record<string, string> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return Object.fromEntries(
    Object.entries(value).filter(([id, color]) => (
      id.length > 0 && typeof color === "string" && /^#[0-9a-f]{6}$/i.test(color)
    )),
  );
}
