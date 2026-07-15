export const WORKSPACE_PANEL_STORAGE_KEY = "portmate.workspacePanels.v1";

export const workspacePanelIds = [
  "explorer",
  "fileManager",
  "sessions",
  "history",
  "sender",
  "statusBar",
] as const;

export type WorkspacePanelId = (typeof workspacePanelIds)[number];
export type WorkspacePanelVisibility = Record<WorkspacePanelId, boolean>;

export const defaultWorkspacePanelVisibility: WorkspacePanelVisibility = {
  explorer: true,
  fileManager: true,
  sessions: true,
  history: true,
  sender: true,
  statusBar: true,
};

export function normalizeWorkspacePanelVisibility(value: unknown): WorkspacePanelVisibility {
  const root = value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
  const source = root?.version === 1 && root.panels && typeof root.panels === "object" && !Array.isArray(root.panels)
    ? root.panels as Record<string, unknown>
    : root;
  return Object.fromEntries(workspacePanelIds.map((id) => [
    id,
    typeof source?.[id] === "boolean" ? source[id] : defaultWorkspacePanelVisibility[id],
  ])) as WorkspacePanelVisibility;
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
