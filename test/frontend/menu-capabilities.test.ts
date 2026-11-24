import { describe, expect, it } from "vitest";
import { menuGroupLayout, menuGroups, menuItemDisabled, menuItemIsDanger, menuItemShortcut, menuSectionsForGroup } from "../../src/menu-capabilities";
import type { MenuCapabilityContext, MenuItem } from "../../src/menu-capabilities";

const ready: MenuCapabilityContext = {
  hasActiveSession: true,
  hasActiveView: true,
  hasSelection: true,
  activeKind: "ssh",
  activeStatus: "connected",
  terminalExportBusy: false,
};

describe("top menu capabilities", () => {
  it("keeps every visible command unique and evaluable", () => {
    const items = menuGroups.flatMap((group) => group.items);
    expect(new Set(items).size).toBe(items.length);
    for (const item of items) {
      expect(typeof menuItemDisabled(item, ready)).toBe("boolean");
    }
    expect(menuGroups.map((group) => group.label)).toEqual(["session", "terminal", "workspace", "tools"]);
    expect(menuGroups.find((group) => group.label === "session")?.items).toEqual([
      "new-session",
      "local-terminal",
      "elevated-local-terminal",
      "import-sessions",
      "duplicate-session",
      "start-session",
      "reconnect-session",
      "close-session-2",
      "session-settings",
    ]);
    expect(menuGroups.find((group) => group.label === "workspace")?.items[0]).toBe("new-workspace-window");
    expect(items).not.toContain("session-search");
    expect(items).not.toContain("copy");
    expect(items).not.toContain("close-pane-3");
  });

  it("separates connection, automation, and management tools without duplicating commands", () => {
    for (const group of menuGroups) {
      const sections = menuSectionsForGroup(group.label, group.items);
      expect(sections.flatMap((section) => section.items), group.label).toEqual([...group.items]);
      expect(sections.every((section) => section.label.length > 0), group.label).toBe(true);
    }
    expect(menuGroupLayout("session")).toBe("single");
    expect(menuGroupLayout("terminal")).toBe("two-column");
    expect(menuGroupLayout("workspace")).toBe("workspace-grid");
    expect(menuGroupLayout("tools")).toBe("two-column");
    expect(menuItemIsDanger("close-session-2")).toBe(true);
    expect(menuItemIsDanger("new-session")).toBe(false);
    const tools = menuGroups.find((group) => group.label === "tools");
    expect(tools).toBeDefined();
    const sections = menuSectionsForGroup("tools", tools?.items ?? []);
    expect(sections.map((section) => section.label)).toEqual(["connection-tools", "automation", "management"]);
    expect(sections[0]?.items).toEqual(["transfer-tasks", "port-forwarding", "easytier-network", "tmux", "serial-analyzer", "sysmon"]);
    expect(sections[2]?.items).toEqual(["terminal-settings", "key-manager", "log-manager", "check-for-updates", "about-portmate"]);
  });

  it("disables session and terminal commands without changing always-available tools", () => {
    const empty = context({
      hasActiveSession: false,
      hasActiveView: false,
      activeKind: null,
      activeStatus: null,
    });
    for (const item of ["session-settings", "start-session", "reconnect-session", "close-session-2", "find", "free-input", "transfer-tasks", "sysmon", "triggers"] as MenuItem[]) {
      expect(menuItemDisabled(item, empty), item).toBe(true);
    }
    for (const item of ["new-session", "import-sessions", "new-workspace-window", "explorer", "terminal-settings", "one-keys", "custom-scripts", "log-manager", "mcp-bridge", "about-portmate"] as MenuItem[]) {
      expect(menuItemDisabled(item, empty), item).toBe(false);
    }
  });

  it("tracks connection and protocol requirements", () => {
    expect(menuItemDisabled("start-session", ready)).toBe(true);
    expect(menuItemDisabled("reconnect-session", ready)).toBe(false);
    expect(menuItemDisabled("close-session-2", ready)).toBe(false);
    expect(menuItemDisabled("port-forwarding", ready)).toBe(false);
    expect(menuItemDisabled("tmux", context({ activeStatus: "disconnected" }))).toBe(true);
    expect(menuItemDisabled("synchronized-input", context({ activeStatus: "reconnecting" }))).toBe(true);
    expect(menuItemDisabled("start-session", context({ activeStatus: "error" }))).toBe(false);
    expect(menuItemDisabled("reconnect-session", context({ activeStatus: "error" }))).toBe(false);
    expect(menuItemDisabled("reconnect-session", context({ activeStatus: "reconnecting" }))).toBe(true);
    expect(menuItemDisabled("serial-analyzer", context({ activeKind: "serial" }))).toBe(false);
    expect(menuItemDisabled("port-forwarding", context({ activeKind: "serial" }))).toBe(true);
  });

  it("locks terminal exports while the active view is exporting", () => {
    const exporting = context({ terminalExportBusy: true });
    expect(menuItemDisabled("export-terminal-text", exporting)).toBe(true);
    expect(menuItemDisabled("export-terminal-text-to", exporting)).toBe(true);
    expect(menuItemDisabled("export-selected-text", exporting)).toBe(true);
    expect(menuItemDisabled("find", exporting)).toBe(false);
  });

  it("disables selected-text export without a terminal selection and all exports without an active view", () => {
    expect(menuItemDisabled("export-selected-text", context({ hasSelection: false }))).toBe(true);
    expect(menuItemDisabled("export-terminal-text", context({ hasSelection: false }))).toBe(false);
    expect(menuItemDisabled("export-terminal-text-to", context({ hasSelection: false }))).toBe(false);
    expect(menuItemDisabled("export-terminal-text", context({ hasActiveView: false }))).toBe(true);
    expect(menuItemDisabled("export-terminal-text-to", context({ hasActiveView: false }))).toBe(true);
    expect(menuItemDisabled("export-selected-text", context({ hasActiveView: false }))).toBe(true);
  });

  it("exposes focus-mode, lock-screen, and export-terminal-text-to with platform-aware shortcut hints", () => {
    expect(menuGroups.find((group) => group.label === "workspace")?.items).toContain("focus-mode");
    expect(menuGroups.find((group) => group.label === "workspace")?.items).toContain("lock-screen");
    expect(menuGroups.find((group) => group.label === "terminal")?.items).toContain("export-terminal-text-to");
    expect(menuItemShortcut("find", undefined, false)).toEqual({ display: "Ctrl+F", aria: "Control+F" });
    expect(menuItemShortcut("find", undefined, true)).toEqual({ display: "Cmd+F", aria: "Meta+F" });
    expect(menuItemShortcut("focus-mode", undefined, false)).toEqual({ display: "Alt+Enter", aria: "Alt+Enter" });
    expect(menuItemShortcut("lock-screen", undefined, false)).toEqual({ display: "Ctrl+Alt+L", aria: "Control+Alt+L" });
    expect(menuItemShortcut("lock-screen", undefined, true)).toEqual({ display: "Cmd+Option+L", aria: "Meta+Alt+L" });
    expect(menuGroupLayout("session")).toBe("single");
    expect(menuGroupLayout("terminal")).toBe("two-column");
    expect(menuGroupLayout("workspace")).toBe("workspace-grid");
    expect(menuGroupLayout("tools")).toBe("two-column");
    expect(menuItemIsDanger("close-session-2")).toBe(true);
    expect(menuItemIsDanger("new-session")).toBe(false);
  });
});

function context(patch: Partial<MenuCapabilityContext>): MenuCapabilityContext {
  return { ...ready, ...patch };
}
