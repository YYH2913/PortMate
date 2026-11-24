import type { SessionKind, SessionStatus } from "./types";
import { formatWorkspaceKeyBinding, type WorkspaceKeymap } from "./workspace-hotkeys";

export const menuGroups = [
  {
    label: "session",
    items: [
      "new-session",
      "local-terminal",
      "elevated-local-terminal",
      "import-sessions",
      "duplicate-session",
      "start-session",
      "reconnect-session",
      "close-session-2",
      "session-settings",
    ],
  },
  {
    label: "terminal",
    items: [
      "find",
      "go-to-line",
      "block-selection",
      "insert-mode",
      "normal-mode",
      "local-mode",
      "local-editing",
      "synchronized-input",
      "free-input",
      "export-terminal-text",
      "export-terminal-text-to",
      "export-selected-text",
    ],
  },
  {
    label: "workspace",
    items: [
      "new-workspace-window",
      "explorer",
      "file-manager",
      "command-history",
      "sysmon-sidebar",
      "send",
      "quick-bar",
      "status-bar",
      "focus-mode",
      "lock-screen",
      "restore-layout",
    ],
  },
  {
    label: "tools",
    items: [
      "transfer-tasks",
      "port-forwarding",
      "easytier-network",
      "tmux",
      "serial-analyzer",
      "sysmon",
      "one-keys",
      "quick-commands",
      "custom-scripts",
      "triggers",
      "mcp-bridge",
      "terminal-settings",
      "key-manager",
      "log-manager",
      "check-for-updates",
      "about-portmate",
    ],
  },
] as const;

export type MenuItem = (typeof menuGroups)[number]["items"][number];

export type MenuSection = {
  label: string;
  items: readonly MenuItem[];
};

const menuSections = {
  session: [
    { label: "create", items: ["new-session", "local-terminal", "elevated-local-terminal", "import-sessions", "duplicate-session"] },
    { label: "connection", items: ["start-session", "reconnect-session", "close-session-2"] },
    { label: "menu-section-config", items: ["session-settings"] },
  ],
  terminal: [
    { label: "edit", items: ["find", "go-to-line", "block-selection"] },
    { label: "input", items: ["insert-mode", "normal-mode", "local-mode", "local-editing", "synchronized-input", "free-input"] },
    { label: "export", items: ["export-terminal-text", "export-terminal-text-to", "export-selected-text"] },
  ],
  workspace: [
    { label: "menu-section-window", items: ["new-workspace-window"] },
    { label: "menu-section-panels", items: ["explorer", "file-manager", "command-history", "sysmon-sidebar", "send", "quick-bar", "status-bar"] },
    { label: "menu-section-view", items: ["focus-mode", "lock-screen", "restore-layout"] },
  ],
  tools: [
    { label: "connection-tools", items: ["transfer-tasks", "port-forwarding", "easytier-network", "tmux", "serial-analyzer", "sysmon"] },
    { label: "automation", items: ["one-keys", "quick-commands", "custom-scripts", "triggers", "mcp-bridge"] },
    { label: "management", items: ["terminal-settings", "key-manager", "log-manager", "check-for-updates", "about-portmate"] },
  ],
} as const satisfies Record<(typeof menuGroups)[number]["label"], readonly MenuSection[]>;

export type MenuGroupLayout = "single" | "two-column" | "workspace-grid";

export function menuGroupLayout(label: string): MenuGroupLayout {
  if (label === "tools" || label === "terminal") return "two-column";
  if (label === "workspace") return "workspace-grid";
  return "single";
}

export function menuItemIsDanger(item: MenuItem): boolean {
  return item === "close-session-2";
}

export function menuSectionsForGroup(label: string, items: readonly MenuItem[]): readonly MenuSection[] {
  return label in menuSections ? menuSections[label as keyof typeof menuSections] : [{ label: "", items }];
}

export type MenuCapabilityContext = {
  hasActiveSession: boolean;
  hasActiveView: boolean;
  hasSelection: boolean;
  activeKind: SessionKind | null;
  activeStatus: SessionStatus | null;
  terminalExportBusy: boolean;
};

export type MenuShortcutHint = {
  display: string;
  aria: string;
};

const activeSessionItems = new Set<MenuItem>([
  "session-settings",
  "export-terminal-text",
  "export-terminal-text-to",
  "export-selected-text",
  "duplicate-session",
  "transfer-tasks",
  "sysmon",
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
  "export-terminal-text",
  "export-terminal-text-to",
  "export-selected-text",
]);

const connectedViewItems = new Set<MenuItem>(["synchronized-input", "free-input"]);

export function menuItemShortcut(
  item: MenuItem,
  workspaceKeymap?: Partial<WorkspaceKeymap>,
  isMac = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform),
): MenuShortcutHint | null {
  switch (item) {
    case "find":
      return isMac ? { display: "Cmd+F", aria: "Meta+F" } : { display: "Ctrl+F", aria: "Control+F" };
    case "focus-mode":
      return { display: "Alt+Enter", aria: "Alt+Enter" };
    case "lock-screen":
      return isMac ? { display: "Cmd+Option+L", aria: "Meta+Alt+L" } : { display: "Ctrl+Alt+L", aria: "Control+Alt+L" };
    case "one-keys": {
      const binding = workspaceKeymap?.["manage-one-keys"]?.trim();
      if (!binding) return null;
      return { display: formatWorkspaceKeyBinding(binding), aria: binding.replace(/\bCtrl\b/g, "Control") };
    }
    default:
      return null;
  }
}

export function menuItemDisabled(item: MenuItem, context: MenuCapabilityContext): boolean {
  if (context.terminalExportBusy && (item === "export-terminal-text" || item === "export-terminal-text-to" || item === "export-selected-text")) return true;
  if (item === "export-selected-text" && !context.hasSelection) return true;
  if (activeSessionItems.has(item) && !context.hasActiveSession) return true;
  if (activeViewItems.has(item) && !context.hasActiveView) return true;
  if (connectedViewItems.has(item)) return !context.hasActiveView || context.activeStatus !== "connected";

  switch (item) {
    case "start-session":
      return !context.hasActiveSession || context.activeStatus === "connecting" || context.activeStatus === "connected" || context.activeStatus === "reconnecting";
    case "reconnect-session":
      return !context.hasActiveSession || context.activeStatus === "connecting" || context.activeStatus === "reconnecting";
    case "close-session-2":
      return !context.hasActiveSession || !["connecting", "connected", "reconnecting"].includes(context.activeStatus ?? "");
    case "port-forwarding":
    case "tmux":
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
