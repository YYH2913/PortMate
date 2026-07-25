export const WORKSPACE_PANEL_STORAGE_KEY = "portmate.workspacePanels.v2";
export const LEGACY_WORKSPACE_PANEL_STORAGE_KEY = "portmate.workspacePanels.v1";

export const workspacePanelIds = [
  "explorer",
  "fileManager",
  "history",
  "sysmon",
  "sender",
  "statusBar",
] as const;

export const workspaceDockPanelIds = [
  "explorer",
  "fileManager",
  "history",
  "sysmon",
  "sender",
] as const;

export const workspaceDockIds = ["left", "right", "bottom"] as const;

export type WorkspacePanelId = (typeof workspacePanelIds)[number];
export type WorkspaceDockPanelId = (typeof workspaceDockPanelIds)[number];
export type WorkspaceDockId = (typeof workspaceDockIds)[number];
export type WorkspacePanelVisibility = Record<WorkspacePanelId, boolean>;
export type WorkspaceDockLayout = Record<WorkspaceDockId, WorkspaceDockPanelId[]> & {
  active: Record<WorkspaceDockId, WorkspaceDockPanelId | null>;
};
export type WorkspaceDockSizes = Record<WorkspaceDockId, number | null>;

export const workspaceDockSizeLimits: Record<WorkspaceDockId, { min: number; max: number; default: number }> = {
  left: { min: 200, max: 720, default: 256 },
  right: { min: 200, max: 720, default: 280 },
  bottom: { min: 120, max: 600, default: 210 },
};

export const defaultWorkspaceDockSizes: WorkspaceDockSizes = {
  left: null,
  right: null,
  bottom: null,
};

export const defaultWorkspacePanelVisibility: WorkspacePanelVisibility = {
  explorer: true,
  fileManager: false,
  history: false,
  sysmon: false,
  sender: false,
  statusBar: true,
};

export const defaultWorkspaceDockLayout: WorkspaceDockLayout = {
  left: ["explorer", "fileManager"],
  right: ["sysmon", "history"],
  bottom: ["sender"],
  active: {
    left: "explorer",
    right: "sysmon",
    bottom: "sender",
  },
};

const legacyWorkspacePanelVisibility: WorkspacePanelVisibility = {
  explorer: true,
  fileManager: true,
  history: true,
  sysmon: false,
  sender: true,
  statusBar: true,
};

export function normalizeWorkspacePanelVisibility(value: unknown): WorkspacePanelVisibility {
  const root = recordValue(value);
  if (!root) return { ...defaultWorkspacePanelVisibility };
  const version = root.version;
  const source = [1, 2, 3, 4, 5, 6, 7].includes(Number(version))
    ? recordValue(root.panels)
    : version === undefined ? root : null;
  const fallback = version === 1 || version === undefined
    ? legacyWorkspacePanelVisibility
    : defaultWorkspacePanelVisibility;
  const normalized = Object.fromEntries(workspacePanelIds.map((id) => [
    id,
    typeof source?.[id] === "boolean" ? source[id] : fallback[id],
  ])) as WorkspacePanelVisibility;
  const untouchedLegacyDefault = version === 1
    && workspacePanelIds.every((id) => normalized[id] === legacyWorkspacePanelVisibility[id]);
  return untouchedLegacyDefault ? { ...defaultWorkspacePanelVisibility } : normalized;
}

export function normalizeWorkspaceDockLayout(value: unknown): WorkspaceDockLayout {
  const root = recordValue(value);
  const source = root && [4, 5, 6, 7].includes(Number(root.version)) ? recordValue(root.docks) : null;
  const seen = new Set<WorkspaceDockPanelId>();
  const order = Object.fromEntries(workspaceDockIds.map((dock) => {
    const candidate = Array.isArray(source?.[dock]) ? source[dock] : defaultWorkspaceDockLayout[dock];
    const panels: WorkspaceDockPanelId[] = [];
    for (const value of candidate) {
      if (typeof value !== "string" || !workspaceDockPanelIds.includes(value as WorkspaceDockPanelId)) continue;
      const panel = value as WorkspaceDockPanelId;
      if (seen.has(panel)) continue;
      seen.add(panel);
      panels.push(panel);
    }
    return [dock, panels];
  })) as Record<WorkspaceDockId, WorkspaceDockPanelId[]>;

  for (const panel of workspaceDockPanelIds) {
    if (seen.has(panel)) continue;
    const fallbackDock = workspaceDockForPanel(defaultWorkspaceDockLayout, panel);
    order[fallbackDock].push(panel);
    seen.add(panel);
  }

  const activeSource = recordValue(source?.active);
  const active = Object.fromEntries(workspaceDockIds.map((dock) => {
    const candidate = activeSource?.[dock];
    return [dock, typeof candidate === "string" && order[dock].includes(candidate as WorkspaceDockPanelId)
      ? candidate as WorkspaceDockPanelId
      : order[dock][0] ?? null];
  })) as Record<WorkspaceDockId, WorkspaceDockPanelId | null>;
  return { ...order, active };
}

export function normalizeWorkspaceDockSizes(value: unknown): WorkspaceDockSizes {
  const root = recordValue(value);
  const source = root && [5, 6, 7].includes(Number(root.version)) ? recordValue(root.sizes) : null;
  return Object.fromEntries(workspaceDockIds.map((dock) => [
    dock,
    normalizeWorkspaceDockSize(dock, source?.[dock]),
  ])) as WorkspaceDockSizes;
}

export function setWorkspaceDockSize(
  current: WorkspaceDockSizes,
  dock: WorkspaceDockId,
  size: number | null,
): WorkspaceDockSizes {
  const normalized = normalizeWorkspaceDockSize(dock, size);
  return current[dock] === normalized ? current : { ...current, [dock]: normalized };
}

export function clampWorkspaceDockSize(dock: WorkspaceDockId, size: number): number {
  const limits = workspaceDockSizeLimits[dock];
  return Math.max(limits.min, Math.min(limits.max, Math.round(size)));
}

export function workspaceDockEffectiveSize(
  sizes: WorkspaceDockSizes,
  dock: WorkspaceDockId,
  visiblePanels: readonly WorkspaceDockPanelId[],
): number {
  const configured = sizes[dock];
  if (configured !== null) return configured;
  if (dock !== "bottom" && visiblePanels.includes("fileManager")) return 360;
  return workspaceDockSizeLimits[dock].default;
}

export function setWorkspacePanelVisibility(
  current: WorkspacePanelVisibility,
  panel: WorkspacePanelId,
  visible: boolean,
): WorkspacePanelVisibility {
  return current[panel] === visible ? current : { ...current, [panel]: visible };
}

export function toggleWorkspacePanelVisibility(
  current: WorkspacePanelVisibility,
  panel: WorkspacePanelId,
): WorkspacePanelVisibility {
  return setWorkspacePanelVisibility(current, panel, !current[panel]);
}

export function activateWorkspaceDockPanel(
  current: WorkspaceDockLayout,
  panel: WorkspaceDockPanelId,
): WorkspaceDockLayout {
  const dock = workspaceDockForPanel(current, panel);
  if (current.active[dock] === panel) return current;
  return { ...current, active: { ...current.active, [dock]: panel } };
}

export function moveWorkspacePanelToDock(
  current: WorkspaceDockLayout,
  panel: WorkspaceDockPanelId,
  targetDock: WorkspaceDockId,
  targetIndex = current[targetDock].length,
): WorkspaceDockLayout {
  const sourceDock = workspaceDockForPanel(current, panel);
  const sourceIndex = current[sourceDock].indexOf(panel);
  const nextOrder = Object.fromEntries(workspaceDockIds.map((dock) => [
    dock,
    current[dock].filter((item) => item !== panel),
  ])) as Record<WorkspaceDockId, WorkspaceDockPanelId[]>;
  const requestedIndex = sourceDock === targetDock && sourceIndex >= 0 && sourceIndex < targetIndex
    ? targetIndex - 1
    : targetIndex;
  const insertAt = Math.max(0, Math.min(Math.trunc(requestedIndex), nextOrder[targetDock].length));
  nextOrder[targetDock].splice(insertAt, 0, panel);

  const active = { ...current.active, [targetDock]: panel };
  for (const dock of workspaceDockIds) {
    if (dock !== targetDock && active[dock] === panel) active[dock] = nextOrder[dock][0] ?? null;
    if (active[dock] && !nextOrder[dock].includes(active[dock])) active[dock] = nextOrder[dock][0] ?? null;
  }
  return { ...nextOrder, active };
}

export function workspaceDockForPanel(
  layout: WorkspaceDockLayout,
  panel: WorkspaceDockPanelId,
): WorkspaceDockId {
  return workspaceDockIds.find((dock) => layout[dock].includes(panel)) ?? "left";
}

export function visibleWorkspaceDockPanels(
  layout: WorkspaceDockLayout,
  visibility: WorkspacePanelVisibility,
  dock: WorkspaceDockId,
): WorkspaceDockPanelId[] {
  return layout[dock].filter((panel) => visibility[panel]);
}

export function activeWorkspaceDockPanel(
  layout: WorkspaceDockLayout,
  visibility: WorkspacePanelVisibility,
  dock: WorkspaceDockId,
): WorkspaceDockPanelId | null {
  const visible = visibleWorkspaceDockPanels(layout, visibility, dock);
  const active = layout.active[dock];
  return active && visible.includes(active) ? active : visible[0] ?? null;
}

export function resolveWorkspacePanelVisibility(
  current: WorkspacePanelVisibility,
  focusMode: boolean,
  preserveStatusBar = false,
): WorkspacePanelVisibility {
  if (!focusMode) return current;
  return Object.fromEntries(workspacePanelIds.map((id) => [
    id,
    id === "statusBar" && preserveStatusBar,
  ])) as WorkspacePanelVisibility;
}

export function isWorkspaceFocusModeShortcut(event: Pick<KeyboardEvent, "altKey" | "code" | "ctrlKey" | "metaKey" | "shiftKey">): boolean {
  return event.code === "Enter" && event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey;
}

function recordValue(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function normalizeWorkspaceDockSize(dock: WorkspaceDockId, value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value)
    ? clampWorkspaceDockSize(dock, value)
    : null;
}
