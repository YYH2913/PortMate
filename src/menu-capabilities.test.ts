import { describe, expect, it } from "vitest";
import { menuGroups, menuItemDisabled, menuSectionsForGroup } from "./menu-capabilities";
import type { MenuCapabilityContext, MenuItem } from "./menu-capabilities";

const ready: MenuCapabilityContext = {
  hasActiveSession: true,
  hasActiveView: true,
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
    expect(items).not.toContain("session-search");
    expect(items).not.toContain("copy");
    expect(items).not.toContain("close-pane-3");
  });

  it("separates connection, automation, and management tools without duplicating commands", () => {
    const tools = menuGroups.find((group) => group.label === "tools");
    expect(tools).toBeDefined();
    const sections = menuSectionsForGroup("tools", tools?.items ?? []);
    expect(sections.map((section) => section.label)).toEqual(["connection-tools", "automation", "management"]);
    expect(sections.flatMap((section) => section.items)).toEqual(tools?.items);
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
    expect(menuItemDisabled("export-selected-text", exporting)).toBe(true);
    expect(menuItemDisabled("find", exporting)).toBe(false);
  });
});

function context(patch: Partial<MenuCapabilityContext>): MenuCapabilityContext {
  return { ...ready, ...patch };
}
