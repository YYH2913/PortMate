export type WorkspaceLayout = "single" | "horizontal" | "vertical";
export type WorkspaceSplitDirection = Exclude<WorkspaceLayout, "single">;
export type WorkspacePaneDirection = "up" | "down" | "left" | "right";
export type WorkspaceSplitPlacement = "first" | "second";

export interface WorkspacePaneNode {
  kind: "pane";
  id: string;
  sessionId: string;
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
  version: 2;
  root: WorkspaceNode | null;
  activePaneId: string;
  activeId: string;
  tabColors: Record<string, string>;
}

export type StartupMode = "none" | "last" | "specific";

export const MAX_WORKSPACE_PANES = 16;
export const MAX_WORKSPACE_DEPTH = 8;
export const MIN_WORKSPACE_SPLIT_RATIO = 0.15;
export const MAX_WORKSPACE_SPLIT_RATIO = 0.85;

export const emptyWorkspaceSnapshot: WorkspaceSnapshot = {
  version: 2,
  root: null,
  activePaneId: "",
  activeId: "",
  tabColors: {},
};

let workspaceIdCounter = 0;

export function createWorkspaceNodeId(kind: "pane" | "split") {
  workspaceIdCounter += 1;
  const uuid = globalThis.crypto?.randomUUID?.();
  return uuid
    ? `${kind}-${uuid}`
    : `${kind}-${Date.now().toString(36)}-${workspaceIdCounter.toString(36)}`;
}

export function createWorkspacePane(sessionId: string, id = createWorkspaceNodeId("pane")): WorkspacePaneNode {
  return { kind: "pane", id, sessionId };
}

export function sanitizeWorkspaceSnapshot(value: unknown): WorkspaceSnapshot {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return { ...emptyWorkspaceSnapshot };
  }
  const source = value as Record<string, unknown>;
  const tabColors = sanitizeTabColors(source.tabColors);
  const requestedActiveId = cleanString(source.activeId, 256);
  const state: SanitizeState = { ids: new Set(), paneCount: 0 };
  const root = source.version === 2 || "root" in source
    ? sanitizeWorkspaceNode(source.root, "root", 0, state)
    : migrateLegacyWorkspace(source, state);
  const withSinglePane = root ?? (
    requestedActiveId ? createSanitizedPane(requestedActiveId, "pane-active", state) : null
  );
  const panes = workspacePaneLeaves(withSinglePane);
  const requestedActivePaneId = cleanString(source.activePaneId, 128);
  const activePane = panes.find((pane) => pane.id === requestedActivePaneId)
    ?? panes.find((pane) => pane.sessionId === requestedActiveId)
    ?? panes[0];
  return {
    version: 2,
    root: withSinglePane,
    activePaneId: activePane?.id ?? "",
    activeId: activePane?.sessionId ?? requestedActiveId,
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
  const tabColors = Object.fromEntries(
    Object.entries(sanitized.tabColors).filter(([id]) => available.has(id)),
  );
  return {
    version: 2,
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
    : workspacePaneLeaves(workspace.root).map((pane) => pane.sessionId);
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
  return workspacePaneLeaves(root).find((pane) => pane.sessionId === sessionId);
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
    return root.id === paneId ? { ...root, sessionId } : root;
  }
  const first = replaceWorkspacePaneSession(root.first, paneId, sessionId);
  const second = replaceWorkspacePaneSession(root.second, paneId, sessionId);
  return first === root.first && second === root.second ? root : { ...root, first: first!, second: second! };
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
    const sessionId = cleanString(source.sessionId, 256);
    if (!sessionId || state.paneCount >= MAX_WORKSPACE_PANES) return null;
    return createSanitizedPane(sessionId, cleanString(source.id, 128) || `pane-${path}`, state);
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

function createSanitizedPane(sessionId: string, id: string, state: SanitizeState): WorkspacePaneNode | null {
  if (state.paneCount >= MAX_WORKSPACE_PANES) return null;
  state.paneCount += 1;
  return createWorkspacePane(sessionId, uniqueNodeId(id, state));
}

function reconcileWorkspaceNode(root: WorkspaceNode | null, available: Set<string>): WorkspaceNode | null {
  if (!root) return null;
  if (root.kind === "pane") return available.has(root.sessionId) ? root : null;
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
