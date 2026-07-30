import type { SessionKind, SessionStatus } from "./types";

export const menuGroups = [
  { label: "会话", items: ["新建会话", "导入会话", "新建工作区窗口", "会话设置", "启动会话", "关闭会话", "复制会话"] },
  { label: "终端", items: ["查找", "跳转到行", "块选择", "Insert 模式", "Normal 模式", "本地模式", "本地编辑", "同步输入", "自由输入", "导出终端文本", "导出选中文本"] },
  { label: "工作区", items: ["资源管理器", "文件管理器", "历史命令", "Sysmon 侧栏", "发送", "快捷栏", "状态栏", "还原布局"] },
  { label: "工具", items: ["传输任务", "终端设置", "OneKeys", "快速命令", "端口转发", "Tmux", "Sysmon", "串口分析器", "触发器", "日志管理", "密钥管理器", "MCP Bridge", "关于 PortMate"] },
] as const;

export type MenuItem = (typeof menuGroups)[number]["items"][number];

export type MenuCapabilityContext = {
  hasActiveSession: boolean;
  hasActiveView: boolean;
  activeKind: SessionKind | null;
  activeStatus: SessionStatus | null;
};

const activeSessionItems = new Set<MenuItem>([
  "会话设置",
  "导出终端文本",
  "导出选中文本",
  "复制会话",
  "传输任务",
  "Sysmon",
  "串口分析器",
  "触发器",
]);

const activeViewItems = new Set<MenuItem>([
  "块选择",
  "查找",
  "跳转到行",
  "Insert 模式",
  "本地模式",
  "Normal 模式",
  "本地编辑",
]);

const connectedViewItems = new Set<MenuItem>(["同步输入", "自由输入"]);

export function menuItemDisabled(item: MenuItem, context: MenuCapabilityContext): boolean {
  if (activeSessionItems.has(item) && !context.hasActiveSession) return true;
  if (activeViewItems.has(item) && !context.hasActiveView) return true;
  if (connectedViewItems.has(item)) return !context.hasActiveView || context.activeStatus !== "connected";

  switch (item) {
    case "启动会话":
      return !context.hasActiveSession || context.activeStatus === "connecting" || context.activeStatus === "connected" || context.activeStatus === "reconnecting";
    case "关闭会话":
      return !context.hasActiveSession || !["connecting", "connected", "reconnecting"].includes(context.activeStatus ?? "");
    case "端口转发":
    case "Tmux":
      return !isSshLike(context.activeKind) || context.activeStatus !== "connected";
    case "串口分析器":
      return context.activeKind !== "serial";
    default:
      return false;
  }
}

function isSshLike(kind: SessionKind | null): boolean {
  return kind === "ssh" || kind === "tmux";
}
