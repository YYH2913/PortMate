import type { SessionKind, SessionStatus } from "./types";

export const menuGroups = [
  { label: "session", items: ["local-terminal", "new-session", "import-sessions", "new-workspace-window", "session-settings", "start-session", "close-session-2", "duplicate-session"] },
  { label: "terminal", items: ["find", "go-to-line", "block-selection", "insert-mode", "normal-mode", "local-mode", "local-editing", "synchronized-input", "free-input", "export-terminal-text", "export-selected-text"] },
  { label: "workspace", items: ["explorer", "file-manager", "command-history", "sysmon-sidebar", "send", "quick-bar", "status-bar", "restore-layout"] },
  { label: "tools", items: ["transfer-tasks", "port-forwarding", "Tmux", "Sysmon", "serial-analyzer", "OneKeys", "quick-commands", "custom-scripts", "triggers", "MCP Bridge", "terminal-settings", "log-manager", "key-manager", "about-portmate"] },
] as const;

export type MenuItem = (typeof menuGroups)[number]["items"][number];

export type MenuSection = {
  label: string;
  items: readonly MenuItem[];
};

const toolMenuSections: readonly MenuSection[] = [
  { label: "connection-tools", items: ["transfer-tasks", "port-forwarding", "Tmux", "Sysmon", "serial-analyzer"] },
  { label: "automation", items: ["OneKeys", "quick-commands", "custom-scripts", "triggers", "MCP Bridge"] },
  { label: "management", items: ["terminal-settings", "log-manager", "key-manager", "about-portmate"] },
];

export function menuSectionsForGroup(label: string, items: readonly MenuItem[]): readonly MenuSection[] {
  return label === "tools" ? toolMenuSections : [{ label: "", items }];
}

export type MenuCapabilityContext = {
  hasActiveSession: boolean;
  hasActiveView: boolean;
  activeKind: SessionKind | null;
  activeStatus: SessionStatus | null;
  terminalExportBusy: boolean;
};

const activeSessionItems = new Set<MenuItem>([
  "session-settings",
  "export-terminal-text",
  "export-selected-text",
  "duplicate-session",
  "transfer-tasks",
  "Sysmon",
  "serial-analyzer",
  "triggers",
]);

const activeViewItems = new Set<MenuItem>([
  "block-selection",
  "find",
  "go-to-line",
  "insert-mode",
  "local-mode",
  "normal-mode",
  "local-editing",
]);

const connectedViewItems = new Set<MenuItem>(["synchronized-input", "free-input"]);

export function menuItemDisabled(item: MenuItem, context: MenuCapabilityContext): boolean {
  if (context.terminalExportBusy && (item === "export-terminal-text" || item === "export-selected-text")) return true;
  if (activeSessionItems.has(item) && !context.hasActiveSession) return true;
  if (activeViewItems.has(item) && !context.hasActiveView) return true;
  if (connectedViewItems.has(item)) return !context.hasActiveView || context.activeStatus !== "connected";

  switch (item) {
    case "start-session":
      return !context.hasActiveSession || context.activeStatus === "connecting" || context.activeStatus === "connected" || context.activeStatus === "reconnecting";
    case "close-session-2":
      return !context.hasActiveSession || !["connecting", "connected", "reconnecting"].includes(context.activeStatus ?? "");
    case "port-forwarding":
    case "Tmux":
      return !isSshLike(context.activeKind) || context.activeStatus !== "connected";
    case "serial-analyzer":
      return context.activeKind !== "serial";
    default:
      return false;
  }
}

function isSshLike(kind: SessionKind | null): boolean {
  return kind === "ssh" || kind === "tmux";
}
