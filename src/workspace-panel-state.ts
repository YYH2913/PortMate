export const WORKSPACE_PANEL_STORAGE_KEY = "portmate.workspacePanels.v2";
export const LEGACY_WORKSPACE_PANEL_STORAGE_KEY = "portmate.workspacePanels.v1";

export const workspacePanelIds = [
  "explorer",
  "fileManager",
  "history",
  "sender",
  "statusBar",
] as const;

export type WorkspacePanelId = (typeof workspacePanelIds)[number];
export type WorkspacePanelVisibility = Record<WorkspacePanelId, boolean>;

export const defaultWorkspacePanelVisibility: WorkspacePanelVisibility = {
  explorer: true,
  fileManager: false,
  history: false,
  sender: false,
  statusBar: true,
};

const legacyWorkspacePanelVisibility: WorkspacePanelVisibility = {
  explorer: true,
  fileManager: true,
  history: true,
  sender: true,
  statusBar: true,
};

export function normalizeWorkspacePanelVisibility(value: unknown): WorkspacePanelVisibility {
  const root = value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
  if (!root) return { ...defaultWorkspacePanelVisibility };
  const version = root?.version;
  const source = (version === 1 || version === 2) && root.panels && typeof root.panels === "object" && !Array.isArray(root.panels)
    ? root.panels as Record<string, unknown>
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

export function setWorkspacePanelVisibility(
  current: WorkspacePanelVisibility,
  panel: WorkspacePanelId,
  visible: boolean,
): WorkspacePanelVisibility {
  if (current[panel] === visible) return current;
  return { ...current, [panel]: visible };
}

export function toggleWorkspacePanelVisibility(
  current: WorkspacePanelVisibility,
  panel: WorkspacePanelId,
): WorkspacePanelVisibility {
  return setWorkspacePanelVisibility(current, panel, !current[panel]);
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
