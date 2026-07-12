export type WorkspaceLayout = "single" | "horizontal" | "vertical";

export interface WorkspaceSnapshot {
  version: 1;
  layout: WorkspaceLayout;
  paneIds: string[];
  activeId: string;
  tabColors: Record<string, string>;
}

export type StartupMode = "none" | "last" | "specific";

export const emptyWorkspaceSnapshot: WorkspaceSnapshot = {
  version: 1,
  layout: "single",
  paneIds: [],
  activeId: "",
  tabColors: {},
};

export function sanitizeWorkspaceSnapshot(value: unknown): WorkspaceSnapshot {
  if (!value || typeof value !== "object") return { ...emptyWorkspaceSnapshot };
  const source = value as Partial<WorkspaceSnapshot>;
  const layout = isWorkspaceLayout(source.layout) ? source.layout : "single";
  const paneIds = uniqueStrings(source.paneIds).slice(0, 4);
  const requestedActiveId = typeof source.activeId === "string" ? source.activeId : "";
  const tabColors = sanitizeTabColors(source.tabColors);
  const split = layout !== "single" && paneIds.length >= 2;
  const activeId = split && !paneIds.includes(requestedActiveId)
    ? paneIds[0]
    : requestedActiveId;
  return {
    version: 1,
    layout: split ? layout : "single",
    paneIds: split ? paneIds : [],
    activeId,
    tabColors,
  };
}

export function reconcileWorkspaceSnapshot(snapshot: WorkspaceSnapshot, sessionIds: string[]): WorkspaceSnapshot {
  const available = new Set(sessionIds);
  const paneIds = snapshot.paneIds.filter((id) => available.has(id));
  const activeId = snapshot.layout !== "single" && paneIds.length >= 2
    ? (paneIds.includes(snapshot.activeId) ? snapshot.activeId : paneIds[0])
    : (available.has(snapshot.activeId) ? snapshot.activeId : sessionIds[0] ?? "");
  const tabColors = Object.fromEntries(
    Object.entries(snapshot.tabColors).filter(([id]) => available.has(id)),
  );
  if (snapshot.layout === "single" || paneIds.length < 2) {
    return { version: 1, layout: "single", paneIds: [], activeId, tabColors };
  }
  return { version: 1, layout: snapshot.layout, paneIds, activeId, tabColors };
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
    : (workspace.layout === "single" ? [workspace.activeId] : workspace.paneIds);
  return requested.filter((id, index) => id && available.has(id) && requested.indexOf(id) === index);
}

function isWorkspaceLayout(value: unknown): value is WorkspaceLayout {
  return value === "single" || value === "horizontal" || value === "vertical";
}

function uniqueStrings(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.filter((item, index): item is string => (
    typeof item === "string" && item.length > 0 && value.indexOf(item) === index
  ));
}

function sanitizeTabColors(value: unknown): Record<string, string> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return Object.fromEntries(
    Object.entries(value).filter(([id, color]) => (
      id.length > 0 && typeof color === "string" && /^#[0-9a-f]{6}$/i.test(color)
    )),
  );
}
