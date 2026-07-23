import { describe, expect, it } from "vitest";
import { menuGroups, menuItemDisabled } from "./menu-capabilities";
import type { MenuCapabilityContext, MenuItem } from "./menu-capabilities";

const ready: MenuCapabilityContext = {
  hasActiveSession: true,
  hasActiveView: true,
  activeKind: "ssh",
  activeStatus: "connected",
};

describe("top menu capabilities", () => {
  it("keeps every visible command unique and evaluable", () => {
    const items = menuGroups.flatMap((group) => group.items);
    expect(new Set(items).size).toBe(items.length);
    for (const item of items) {
      expect(typeof menuItemDisabled(item, ready)).toBe("boolean");
    }
    expect(menuGroups.map((group) => group.label)).toEqual(["会话", "终端", "工作区", "工具"]);
    expect(items).not.toContain("会话搜索");
    expect(items).not.toContain("复制");
    expect(items).not.toContain("关闭窗格");
  });

  it("disables session and terminal commands without changing always-available tools", () => {
    const empty = context({
      hasActiveSession: false,
      hasActiveView: false,
      activeKind: null,
      activeStatus: null,
    });
    for (const item of ["会话设置", "启动会话", "关闭会话", "查找", "自由输入", "传输任务", "Sysmon", "触发器"] as MenuItem[]) {
      expect(menuItemDisabled(item, empty), item).toBe(true);
    }
    for (const item of ["新建会话", "导入 OpenSSH 配置", "导入 PuTTY 配置", "新建工作区窗口", "资源管理器", "终端设置", "OneKeys", "日志管理", "MCP Bridge", "关于 PortMate"] as MenuItem[]) {
      expect(menuItemDisabled(item, empty), item).toBe(false);
    }
  });

  it("tracks connection and protocol requirements", () => {
    expect(menuItemDisabled("启动会话", ready)).toBe(true);
    expect(menuItemDisabled("关闭会话", ready)).toBe(false);
    expect(menuItemDisabled("端口转发", ready)).toBe(false);
    expect(menuItemDisabled("Tmux", context({ activeStatus: "disconnected" }))).toBe(true);
    expect(menuItemDisabled("同步输入", context({ activeStatus: "reconnecting" }))).toBe(true);
    expect(menuItemDisabled("启动会话", context({ activeStatus: "error" }))).toBe(false);
    expect(menuItemDisabled("串口分析器", context({ activeKind: "serial" }))).toBe(false);
    expect(menuItemDisabled("端口转发", context({ activeKind: "serial" }))).toBe(true);
  });
});

function context(patch: Partial<MenuCapabilityContext>): MenuCapabilityContext {
  return { ...ready, ...patch };
}
