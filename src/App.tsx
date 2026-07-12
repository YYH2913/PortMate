import { useEffect, useMemo, useRef, useState } from "react";
import type { DragEvent as ReactDragEvent, FormEvent, MouseEvent as ReactMouseEvent, ReactNode } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { Terminal as XTerm } from "@xterm/xterm";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  ArrowUp,
  ChevronDown,
  ChevronRight,
  Copy,
  File,
  Folder,
  KeyRound,
  Lock,
  Play,
  Plus,
  RefreshCw,
  Search,
  Settings,
  Square,
  Trash2,
  UserPlus,
  X,
} from "lucide-react";
import { callBackend, emptyAudit, emptyGrants, emptyHostKeys, emptyLogs, emptySessions, emptyTransfers, invokeBackend, isBackendAvailable } from "./api";
import { mergeTransfers } from "./transfer-state";
import type { AuditRecord, AuthMethod, ConnectionConfig, ExternalDropResult, FileEntry, FileProperties, HostKeyObservation, HostKeyPolicy, HostKeyScanResult, HostKeyStore, IdentityRef, JumpHop, McpGrant, McpHttpConfig, McpHttpTokenResponse, McpScope, SessionEvent, SessionKind, SessionProfile, SessionStatus, SessionSummary, SysmonSnapshot, TmuxState, TransferTask, TriggerSpec, TunnelSpec, TunnelStatus, TrustedHostKey } from "./types";

const menuGroups = [
  { label: "会话", items: ["新建会话", "会话设置", "启动会话", "关闭会话", "复制标签", "还原布局"] },
  { label: "编辑", items: ["复制", "粘贴", "粘贴确认", "全选", "查找"] },
  { label: "搜索", items: ["会话搜索", "日志搜索", "在线搜索"] },
  { label: "选择", items: ["选择全部", "块选择", "清除选择"] },
  { label: "转到", items: ["上一个标签", "下一个标签", "跳转到行"] },
  { label: "查看", items: ["资源管理器", "文件管理器", "会话", "历史命令", "发送", "状态栏"] },
  { label: "模式", items: ["远程模式", "本地模式", "同步输入", "自由输入", "锁屏"] },
  { label: "传输", items: ["SFTP/SCP 传输", "X/Y/ZModem"] },
  { label: "工具", items: ["终端设置", "端口转发", "Tmux", "Sysmon", "触发器", "密钥管理器", "MCP Bridge"] },
  { label: "窗口", items: ["水平拆分", "垂直拆分", "关闭窗格"] },
  { label: "帮助", items: ["关于 PortMate"] },
];

const terminalSettingTree = [
  { label: "应用" },
  { label: "外观" },
  { label: "代理" },
  { label: "安全" },
  { label: "标签" },
  { label: "终端", children: ["Auto Completion", "命令历史", "鼠标追踪"] },
  { label: "文本", children: ["二进制", "插入符", "字体", "高亮", "换行"] },
  { label: "小部件", children: ["文件管理器", "快捷栏"] },
  { label: "X Server", children: ["扩展"] },
] as const;

const protocolTabs = ["Shell", "SSH", "Tmux", "Telnet", "Tcp", "Serial"] as const;

type SettingsDialog = "terminal" | "session" | null;
type UtilityDialog = "transfer" | "tunnel" | "tmux" | "search" | "keys" | "mcp" | null;
type ProtocolTab = (typeof protocolTabs)[number];
type SessionTreeNode = { label: string; children?: readonly string[] };
type TerminalPrefs = ReturnType<typeof createTerminalPrefs>;
type SessionPrefs = ReturnType<typeof createSessionPrefs>;
type ConnectionCredentials = { username: string | null; password: string | null; passphrase: string | null; savePassword: boolean; savePassphrase: boolean };
type NoticeState = { title: string; message: string } | null;
type SearchDialogState = { mode: "sessions" | "logs"; query: string };
type HostKeyDecisionValue = "trust-once" | "append-to-profile" | "append-to-project" | "replace-for-profile";
type HostKeyEditDraft = {
  keyId: string;
  profileId: string;
  alias: string;
  host: string;
  port: number;
  scope: TrustedHostKey["scope"];
  label: string;
};
type ClientIdentityGroupBy = "profile" | "source";
type ClientIdentityItem = {
  selectionId: string;
  profileId: string;
  profileName: string;
  identity: IdentityRef;
  jumpInUse: boolean;
};
type HostKeyPromptState = {
  profile: SessionProfile;
  message: string;
  scan: HostKeyScanResult | null;
  scanError: string | null;
  busy: boolean;
};
type WorkspaceLayout = "single" | "horizontal" | "vertical";
type SendMode = "text" | "hex";
type SendTarget = "active" | "panes" | "connected";
type ContextMenuState = { x: number; y: number; sessionId: string | null } | null;
type CredentialPromptState = {
  target: string;
  initialUsername: string;
  hasIdentityFiles: boolean;
  hasSavedPassword: boolean;
  hasSavedPassphrase: boolean;
  needsPassword: boolean;
  authOrder: AuthMethod[];
};

const sharedSessionTree: readonly SessionTreeNode[] = [
  { label: "会话" },
  { label: "终端", children: ["Bell", "模式", "键盘", "安全", "日志"] },
  { label: "窗口", children: ["选择"] },
  { label: "自动化", children: ["触发器"] },
];

const sessionSettingTrees: Record<ProtocolTab, readonly SessionTreeNode[]> = {
  Shell: [...sharedSessionTree, { label: "Shell", children: ["进程"] }],
  SSH: [...sharedSessionTree, { label: "SSH", children: ["连接", "代理", "验证", "代理人", "密码", "密钥交换", "MAC 哈希", "公钥", "SFTP", "X11"] }, { label: "X/Y/Z Modem" }],
  Tmux: [...sharedSessionTree, { label: "Tmux", children: ["连接", "代理", "验证", "代理人", "密码", "密钥交换", "MAC 哈希", "公钥", "SFTP", "X11"] }, { label: "X/Y/Z Modem" }],
  Telnet: [...sharedSessionTree, { label: "Telnet", children: ["连接", "代理"] }, { label: "X/Y/Z Modem" }],
  Tcp: [...sharedSessionTree, { label: "Tcp", children: ["连接", "代理"] }, { label: "X/Y/Z Modem" }],
  Serial: [...sharedSessionTree, { label: "串口", children: ["协议"] }, { label: "X/Y/Z Modem" }],
};

const tabColorChoices = [
  { label: "青色", value: "#5eead4" },
  { label: "蓝色", value: "#68a7ff" },
  { label: "紫色", value: "#a78bfa" },
  { label: "琥珀", value: "#f4b860" },
  { label: "红色", value: "#f87171" },
  { label: "绿色", value: "#37d67a" },
];

const portmateTerminalTheme = {
  background: "#0d1117",
  foreground: "#d7e1eb",
  cursor: "#5eead4",
  cursorAccent: "#0d1117",
  selectionBackground: "#284457",
  selectionForeground: "#f8fafc",
  black: "#0d1117",
  red: "#f87171",
  green: "#37d67a",
  yellow: "#f4b860",
  blue: "#68a7ff",
  magenta: "#c084fc",
  cyan: "#5eead4",
  white: "#d7e1eb",
  brightBlack: "#6b7280",
  brightRed: "#ff8a8a",
  brightGreen: "#86efac",
  brightYellow: "#fde047",
  brightBlue: "#93c5fd",
  brightMagenta: "#d8b4fe",
  brightCyan: "#67e8f9",
  brightWhite: "#ffffff",
  extendedAnsi: createXterm256Palette(),
};

function createXterm256Palette() {
  const toHex = (value: number) => value.toString(16).padStart(2, "0");
  const rgb = (red: number, green: number, blue: number) => `#${toHex(red)}${toHex(green)}${toHex(blue)}`;
  const palette: string[] = [];
  const steps = [0, 95, 135, 175, 215, 255];
  for (const red of steps) {
    for (const green of steps) {
      for (const blue of steps) {
        palette.push(rgb(red, green, blue));
      }
    }
  }
  for (let index = 0; index < 24; index += 1) {
    const level = 8 + index * 10;
    palette.push(rgb(level, level, level));
  }
  return palette;
}

export default function App() {
  const [sessions, setSessions] = useState<SessionSummary[]>(emptySessions);
  const [logs, setLogs] = useState<Record<string, SessionEvent[]>>(emptyLogs);
  const [transfers, setTransfers] = useState<TransferTask[]>(emptyTransfers);
  const [audit, setAudit] = useState<AuditRecord[]>(emptyAudit);
  const [grants, setGrants] = useState<McpGrant[]>(emptyGrants);
  const [hostKeys, setHostKeys] = useState<HostKeyStore>(emptyHostKeys);
  const [serialPorts, setSerialPorts] = useState<string[]>([]);
  const [activeId, setActiveId] = useState("");
  const [openMenu, setOpenMenu] = useState<string | null>(null);
  const [dialog, setDialog] = useState<SettingsDialog>(null);
  const [utilityDialog, setUtilityDialog] = useState<UtilityDialog>(null);
  const [searchDialog, setSearchDialog] = useState<SearchDialogState>({ mode: "sessions", query: "" });
  const [draft, setDraft] = useState<SessionProfile>(() => createSessionDraft());
  const [sendText, setSendText] = useState("");
  const [sendMode, setSendMode] = useState<SendMode>("text");
  const [sendCount, setSendCount] = useState(1);
  const [sendIntervalMs, setSendIntervalMs] = useState(1000);
  const [sendTarget, setSendTarget] = useState<SendTarget>("active");
  const [sendBusy, setSendBusy] = useState(false);
  const [syncInput, setSyncInput] = useState(() => loadLocalValue("portmate.syncInput", false));
  const [commandHistory, setCommandHistory] = useState<string[]>(() => loadLocalValue("portmate.commandHistory", []));
  const [notice, setNotice] = useState<NoticeState>(null);
  const [hostKeyPrompt, setHostKeyPrompt] = useState<HostKeyPromptState | null>(null);
  const [sessionSettingsSection, setSessionSettingsSection] = useState("会话");
  const [credentialPrompt, setCredentialPrompt] = useState<CredentialPromptState | null>(null);
  const [workspaceLayout, setWorkspaceLayout] = useState<WorkspaceLayout>(() => loadLocalValue("portmate.workspaceLayout", "single"));
  const [paneIds, setPaneIds] = useState<string[]>(() => loadLocalValue("portmate.paneIds", []));
  const [blockSelection, setBlockSelection] = useState(false);
  const [contextMenu, setContextMenu] = useState<ContextMenuState>(null);
  const [tabColors, setTabColors] = useState<Record<string, string>>(() => loadLocalValue("portmate.tabColors", {}));
  const credentialResolverRef = useRef<((credentials: ConnectionCredentials | null) => void) | null>(null);
  const logSignatureRef = useRef<Record<string, string>>({});
  const sessionsSignatureRef = useRef("");

  const active = sessions.find((session) => session.profile.id === activeId);
  const activeStatus = active?.runtime.status;
  const activeSerial = active?.profile.connection.kind === "serial" ? active.profile.connection : null;

  useEffect(() => {
    void refresh();
  }, []);

  useEffect(() => {
    saveLocalValue("portmate.workspaceLayout", workspaceLayout);
    saveLocalValue("portmate.paneIds", paneIds);
  }, [workspaceLayout, paneIds]);

  useEffect(() => {
    saveLocalValue("portmate.syncInput", syncInput);
  }, [syncInput]);

  useEffect(() => {
    saveLocalValue("portmate.commandHistory", commandHistory.slice(0, 200));
  }, [commandHistory]);

  useEffect(() => {
    saveLocalValue("portmate.tabColors", tabColors);
  }, [tabColors]);

  useEffect(() => {
    const preventNativeContextMenu = (event: MouseEvent) => {
      event.preventDefault();
    };
    window.addEventListener("contextmenu", preventNativeContextMenu, { capture: true });
    return () => window.removeEventListener("contextmenu", preventNativeContextMenu, { capture: true });
  }, []);

  useEffect(() => {
    if (!contextMenu) return;
    const close = () => setContextMenu(null);
    window.addEventListener("scroll", close, true);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("scroll", close, true);
      window.removeEventListener("resize", close);
    };
  }, [contextMenu]);

  useEffect(() => {
    if (!activeId || !isBackendAvailable()) return;
    if (!activeStatus || activeStatus === "disconnected") return;
    void refreshActiveLog(activeId);
    const timer = window.setInterval(() => {
      void refreshActiveLog(activeId);
    }, activeStatus === "connected" ? 2000 : 1200);
    return () => window.clearInterval(timer);
  }, [activeId, activeStatus]);

  useEffect(() => {
    if (!activeId || !isBackendAvailable()) return;
    if (!activeStatus || activeStatus === "disconnected") return;
    const timer = window.setInterval(() => {
      void refreshSessionSummaries();
    }, 2500);
    return () => window.clearInterval(timer);
  }, [activeId, activeStatus]);

  useEffect(() => {
    if (!isBackendAvailable()) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<TransferTask>("portmate-transfer-task", (event) => {
      if (disposed) return;
      setTransfers((current) => mergeTransfers(current, event.payload));
    })
      .then((nextUnlisten) => {
        if (disposed) {
          nextUnlisten();
          return;
        }
        unlisten = nextUnlisten;
      })
      .catch(() => {});

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  async function refresh() {
    const nextSessions = await callBackend("list_sessions", {}, loadLocalSessionSummaries());
    setSessions(nextSessions);
    setTransfers(await callBackend("list_transfers", {}, emptyTransfers));
    setAudit(await callBackend("list_mcp_audit", {}, emptyAudit));
    setGrants(await callBackend("list_mcp_grants", {}, emptyGrants));
    setHostKeys(await callBackend("list_host_keys", {}, emptyHostKeys));
    setSerialPorts(await callBackend("list_serial_ports", {}, []));
    setActiveId((current) => nextSessions.find((session) => session.profile.id === current)?.profile.id ?? nextSessions[0]?.profile.id ?? "");

    const nextLogs: Record<string, SessionEvent[]> = {};
    for (const session of nextSessions) {
      nextLogs[session.profile.id] = await callBackend("tail_log", { sessionId: session.profile.id, limit: 160 }, []);
    }
    setLogs(nextLogs);
  }

  async function refreshActiveLog(sessionId: string) {
    const nextLog = await callBackend("tail_log", { sessionId, limit: 600 }, []);
    const signature = logSignature(nextLog);
    if (logSignatureRef.current[sessionId] === signature) {
      return;
    }
    logSignatureRef.current[sessionId] = signature;
    setLogs((current) => ({ ...current, [sessionId]: nextLog }));
  }

  async function refreshSessionSummaries() {
    const nextSessions = await callBackend("list_sessions", {}, []);
    if (nextSessions.length) {
      const signature = sessionsSignature(nextSessions);
      if (sessionsSignatureRef.current === signature) {
        return;
      }
      sessionsSignatureRef.current = signature;
      setSessions(nextSessions);
      saveLocalSessionSummaries(nextSessions);
    }
  }
  const sessionGroups = useMemo(
    () =>
      sessions.reduce<Record<string, SessionSummary[]>>((acc, session) => {
        acc[session.profile.group || "Sessions"] ??= [];
        acc[session.profile.group || "Sessions"].push(session);
        return acc;
      }, {}),
    [sessions],
  );

  const paneSessions = useMemo(() => {
    const ids = workspaceLayout === "single" ? [activeId] : paneIds.length ? paneIds : [activeId];
    return ids
      .map((id) => sessions.find((session) => session.profile.id === id))
      .filter((session): session is SessionSummary => Boolean(session));
  }, [activeId, paneIds, sessions, workspaceLayout]);

function handleMenuAction(item: string) {
    if (item === "终端设置") {
      setDialog("terminal");
      return;
    }
    if (item === "MCP Bridge") {
      setUtilityDialog("mcp");
      return;
    }
    if (item === "关于 PortMate") {
      setNotice({ title: "关于 PortMate", message: "PortMate 是面向串口、SSH 和 MCP 会话控制的桌面终端工作台。" });
      return;
    }
    if (item === "会话搜索" || item === "查找") {
      setSearchDialog({ mode: "sessions", query: "" });
      setUtilityDialog("search");
      return;
    }
    if (item === "日志搜索") {
      setSearchDialog({ mode: "logs", query: "" });
      setUtilityDialog("search");
      return;
    }
    if (item === "在线搜索") {
      const selectedText = window.getSelection()?.toString().trim() || active?.lastLine || "";
      window.open(`https://www.google.com/search?q=${encodeURIComponent(selectedText)}`, "_blank", "noopener,noreferrer");
      return;
    }
    if (item === "复制" || item === "全选" || item === "选择全部") {
      document.execCommand(item === "复制" ? "copy" : "selectAll");
      return;
    }
    if (item === "粘贴" || item === "粘贴确认") {
      void navigator.clipboard?.readText().then((text) => {
        if (!active || !text) return;
        if (item === "粘贴确认" && !window.confirm(`粘贴 ${text.length} 个字符到 ${active.profile.name}?`)) return;
        void sendTerminalInput(active.profile.id, text);
      }).catch((error) => setNotice({ title: item, message: formatError(error) }));
      return;
    }
    if (item === "上一个标签" || item === "下一个标签") {
      if (!sessions.length) return;
      const current = Math.max(0, sessions.findIndex((session) => session.profile.id === activeId));
      const offset = item === "上一个标签" ? -1 : 1;
      const next = (current + offset + sessions.length) % sessions.length;
      setActiveId(sessions[next].profile.id);
      return;
    }
    if (item === "跳转到行") {
      setSearchDialog({ mode: "logs", query: "" });
      setUtilityDialog("search");
      return;
    }
    if (["资源管理器", "文件管理器", "会话", "历史命令", "发送", "状态栏", "远程模式", "本地模式", "自由输入", "锁屏"].includes(item)) {
      setNotice({ title: item, message: "该工作区视图已经显示在当前桌面布局中。" });
      return;
    }
    if (item === "同步输入") {
      setSyncInput((current) => !current);
      return;
    }
    if (item === "水平拆分" || item === "垂直拆分") {
      splitWorkspace(item === "水平拆分" ? "horizontal" : "vertical");
      return;
    }
    if (item === "关闭窗格") {
      closeWorkspacePane();
      return;
    }
    if (item === "块选择") {
      setBlockSelection((current) => !current);
      return;
    }
    if (item === "清除选择") {
      window.getSelection()?.removeAllRanges();
      setBlockSelection(false);
      return;
    }
    if (item === "新建会话") {
      setDraft(createSessionDraft());
      setSessionSettingsSection("会话");
      setDialog("session");
      return;
    }
    if (item === "启动会话") {
      void connectSession();
      return;
    }
    if (item === "关闭会话") {
      void disconnectSession();
      return;
    }
    if (item === "会话设置") {
      setDraft(active?.profile ?? createSessionDraft());
      setSessionSettingsSection("会话");
      setDialog("session");
      return;
    }
    if (["端口转发", "触发器", "密钥管理器"].includes(item)) {
      if (item === "端口转发") {
        if (!active || !isSshLikeProfile(active.profile)) {
          setNotice({ title: "端口转发", message: "请选择一个已保存并已连接的 SSH/Tmux 会话后再创建 tunnel。" });
          return;
        }
        setUtilityDialog("tunnel");
        return;
      }
      if (item === "触发器") {
        setDraft(active?.profile ?? createSessionDraft());
        setSessionSettingsSection("触发器");
        setDialog("session");
        return;
      }
      setUtilityDialog("keys");
      return;
    }
    if (item === "Sysmon") {
      void refreshSysmon();
      return;
    }
    if (item === "Tmux") {
      if (!active || !isSshLikeProfile(active.profile) || active.runtime.status !== "connected") {
        setNotice({ title: "Tmux", message: "请选择一个已连接的 SSH/Tmux 会话后再管理 tmux。" });
        return;
      }
      setUtilityDialog("tmux");
      return;
    }
    if (item === "SFTP/SCP 传输" || item === "X/Y/ZModem") {
      if (!active) {
        setNotice({ title: item, message: "请先选择一个会话。" });
        return;
      }
      setUtilityDialog("transfer");
      return;
    }
    if (item === "复制标签") {
      duplicateSessionFromContext();
      return;
    }
    if (item === "还原布局") {
      void refresh();
      return;
    }

    setNotice({ title: item, message: "未识别的菜单项。" });
  }

  function openAppContextMenu(event: ReactMouseEvent, sessionId?: string) {
    event.preventDefault();
    event.stopPropagation();
    const nextSessionId = sessionId ?? (activeId || sessions[0]?.profile.id || null);
    if (sessionId) {
      setActiveId(sessionId);
    }
    setOpenMenu(null);
    setContextMenu({ x: event.clientX, y: event.clientY, sessionId: nextSessionId });
  }

  function contextSession(sessionId?: string | null) {
    return sessions.find((session) => session.profile.id === (sessionId ?? contextMenu?.sessionId ?? activeId));
  }

  async function renameSessionFromContext(sessionId?: string | null) {
    const session = contextSession(sessionId);
    if (!session) return;
    const nextName = window.prompt("标签名称", session.profile.name);
    if (!nextName?.trim()) return;
    const saved = await saveProfile({ ...session.profile, name: nextName.trim() });
    applySavedSession(saved);
  }

  async function moveSessionToGroupFromContext(sessionId?: string | null) {
    const session = contextSession(sessionId);
    if (!session) return;
    const nextGroup = window.prompt("移动到分组", session.profile.group || "Sessions");
    if (nextGroup === null) return;
    const saved = await saveProfile({ ...session.profile, group: nextGroup.trim() || "Sessions" });
    applySavedSession(saved);
  }

  async function saveSessionFromContext(sessionId?: string | null) {
    const session = contextSession(sessionId);
    if (!session) return;
    const saved = await saveProfile(prepareSessionProfile(session.profile));
    applySavedSession(saved);
    setNotice({ title: "保存会话", message: `已保存 ${saved.profile.name}` });
  }

  function duplicateSessionFromContext(sessionId?: string | null) {
    const session = contextSession(sessionId);
    if (!session) {
      openNewSessionDialog();
      return;
    }
    const duplicate = cloneSessionProfile(session.profile);
    duplicate.id = createSessionId();
    duplicate.name = `${session.profile.name} copy`;
    duplicate.connection = isolateDuplicatedConnection(duplicate.id, duplicate.connection);
    setDraft(duplicate);
    setSessionSettingsSection("会话");
    setDialog("session");
  }

  function openSessionSettingsFromContext(sessionId?: string | null) {
    const session = contextSession(sessionId);
    setDraft(session?.profile ?? createSessionDraft());
    setSessionSettingsSection("会话");
    setDialog("session");
  }

  function copySessionNameFromContext(sessionId?: string | null) {
    const session = contextSession(sessionId);
    if (!session) return;
    void navigator.clipboard?.writeText(session.profile.name);
  }

  function copySessionUrlFromContext(sessionId?: string | null) {
    const session = contextSession(sessionId);
    if (!session) return;
    const url = `portmate://sessions/${encodeURIComponent(session.profile.id)}?kind=${encodeURIComponent(session.profile.kind)}&endpoint=${encodeURIComponent(describeProfileEndpoint(session.profile))}`;
    void navigator.clipboard?.writeText(url);
  }

  function setTabColorFromContext(sessionId: string | null | undefined, color: string) {
    const session = contextSession(sessionId);
    if (!session) return;
    setTabColors((current) => ({ ...current, [session.profile.id]: color }));
  }

  async function pasteFromClipboardIntoContext(sessionId?: string | null) {
    const session = contextSession(sessionId);
    if (!session) return;
    const text = await navigator.clipboard?.readText().catch(() => "");
    if (text) await routeTerminalInput(session.profile.id, text);
  }

  async function closeSessionsByIds(ids: string[]) {
    const failed: string[] = [];
    for (const id of ids) {
      try {
        await disconnectSession(id);
      } catch {
        failed.push(id);
      }
    }
    if (failed.length) {
      setNotice({ title: "关闭视图", message: `${failed.length} 个会话关闭失败，其余已关闭。` });
    }
  }

  function closeSideSessionsFromContext(sessionId?: string | null) {
    const target = contextSession(sessionId);
    if (!target) return;
    const index = sessions.findIndex((session) => session.profile.id === target.profile.id);
    if (index < 0) return;
    const rightIds = sessions.slice(index + 1).map((session) => session.profile.id);
    void closeSessionsByIds(rightIds);
  }

  function handleContextMenuAction(action: string, sessionId?: string | null) {
    setContextMenu(null);
    const target = contextSession(sessionId);
    switch (action) {
      case "sync-on":
        setSyncInput(true);
        return;
      case "sync-off":
        setSyncInput(false);
        return;
      case "rename":
        void renameSessionFromContext(sessionId);
        return;
      case "duplicate":
        duplicateSessionFromContext(sessionId);
        return;
      case "paste":
        void pasteFromClipboardIntoContext(sessionId);
        return;
      case "copy-name":
        copySessionNameFromContext(sessionId);
        return;
      case "copy-url":
        copySessionUrlFromContext(sessionId);
        return;
      case "reconnect":
        if (target) void connectSession(target.profile.id);
        return;
      case "save":
        void saveSessionFromContext(sessionId);
        return;
      case "split-h":
        splitWorkspace("horizontal");
        return;
      case "split-v":
        splitWorkspace("vertical");
        return;
      case "move-group":
        void moveSessionToGroupFromContext(sessionId);
        return;
      case "close":
        if (target) void disconnectSession(target.profile.id);
        return;
      case "close-all":
        void closeSessionsByIds(sessions.map((session) => session.profile.id));
        return;
      case "close-inactive":
        void closeSessionsByIds(sessions.filter((session) => session.profile.id !== activeId).map((session) => session.profile.id));
        return;
      case "close-side":
        closeSideSessionsFromContext(sessionId);
        return;
      case "settings":
        openSessionSettingsFromContext(sessionId);
        return;
      default:
        return;
    }
  }

  function openNewSessionDialog() {
    setDraft(createSessionDraft());
    setSessionSettingsSection("会话");
    setDialog("session");
  }

  function activateSession(sessionId: string) {
    setActiveId(sessionId);
    if (workspaceLayout !== "single") {
      setPaneIds((current) => {
        if (!current.length || current.includes(sessionId)) return current;
        return [sessionId, ...current.slice(1)];
      });
    }
  }

  function splitWorkspace(layout: Exclude<WorkspaceLayout, "single">) {
    const primaryId = activeId || sessions[0]?.profile.id;
    if (!primaryId) {
      openNewSessionDialog();
      return;
    }
    const currentIds = workspaceLayout === "single" || !paneIds.length ? [primaryId] : paneIds;
    const nextId = sessions.find((session) => !currentIds.includes(session.profile.id))?.profile.id ?? primaryId;
    const nextIds = currentIds.length >= 4 ? currentIds : [...currentIds, nextId];
    setWorkspaceLayout(layout);
    setPaneIds(nextIds);
    setActiveId(primaryId);
  }

  function closeWorkspacePane(paneIndex?: number) {
    const currentIds = paneIds.length ? paneIds : activeId ? [activeId] : [];
    if (!currentIds.length) return;
    const nextIds = typeof paneIndex === "number" ? currentIds.filter((_, index) => index !== paneIndex) : currentIds.slice(0, -1);
    if (nextIds.length <= 1) {
      setWorkspaceLayout("single");
      setPaneIds([]);
      if (nextIds[0]) setActiveId(nextIds[0]);
      return;
    }
    setPaneIds(nextIds);
    setActiveId(nextIds[0]);
  }

  async function saveDraft() {
    const profile = prepareSessionProfile(draft);
    try {
      const saved = await saveProfile(profile);
      applySavedSession(saved);
      setDraft(saved.profile);
      setDialog(null);
    } catch (error) {
      setNotice({ title: "保存会话失败", message: formatError(error) });
    }
  }

  async function saveDraftAndConnect() {
    const profile = prepareSessionProfile(draft);
    try {
      const saved = await saveProfile(profile);
      applySavedSession(saved);
      setDraft(saved.profile);
      setDialog(null);
      await connectSession(saved.profile.id, saved);
    } catch (error) {
      setNotice({ title: "保存会话失败", message: formatError(error) });
    }
  }

  async function saveProfile(profile: SessionProfile) {
    if (isBackendAvailable()) {
      return invokeBackend<SessionSummary>("save_session_profile", { profile });
    }
    return createSessionSummary(profile);
  }

  function applySavedSession(saved: SessionSummary) {
    setActiveId(saved.profile.id);
    setSessions((current) => {
      const nextSessions = mergeSessionSummaries(current, saved);
      saveLocalSessionSummaries(nextSessions);
      return nextSessions;
    });
  }

  async function connectSession(sessionId = activeId, sessionOverride?: SessionSummary) {
    const session = sessionOverride ?? sessions.find((item) => item.profile.id === sessionId);
    if (!session) return;

    const credentials = await requestSessionCredentials(session.profile);
    if (credentials === null) return;
    let profileForConnect: SessionProfile;
    try {
      profileForConnect = await persistConnectionSecrets(applyConnectionCredentials(prepareSessionProfile(session.profile), credentials), credentials);
    } catch (error) {
      setNotice({ title: "保存凭据失败", message: formatError(error) });
      return;
    }

    const connecting = setSessionStatus({ ...session, profile: profileForConnect }, "connecting");
    setSessions((current) => mergeSessionSummaries(current, connecting));
    setActiveId(profileForConnect.id);

    try {
      const persisted = await saveProfile(profileForConnect);
      applySavedSession(persisted);
      const saved = isBackendAvailable()
        ? await invokeBackend<SessionSummary>("open_session", { sessionId: persisted.profile.id, password: credentials.password, passphrase: credentials.passphrase })
        : setSessionStatus(persisted, "connected");
      const fallbackLog = [...(logs[persisted.profile.id] ?? []), createLocalSystemEvent(saved.profile, `PortMate: connected to ${describeProfileEndpoint(saved.profile)}`)];
      const nextLog = await callBackend("tail_log", { sessionId: persisted.profile.id, limit: 600 }, fallbackLog);

      setLogs((current) => ({ ...current, [persisted.profile.id]: nextLog }));
      setSessions((current) => {
        const nextSessions = mergeSessionSummaries(current, saved);
        saveLocalSessionSummaries(nextSessions);
        return nextSessions;
      });
    } catch (error) {
      const message = formatError(error);
      const failed = setSessionStatus({ ...session, profile: profileForConnect }, "error");
      const backendLog = await callBackend("tail_log", { sessionId: profileForConnect.id, limit: 600 }, []);
      const errorText = `PortMate: connection failed: ${message}`;
      const nextLog = backendLog.length ? backendLog : [...(logs[profileForConnect.id] ?? []), createLocalSystemEvent(profileForConnect, errorText)];
      setLogs((current) => ({ ...current, [profileForConnect.id]: nextLog }));
      setSessions((current) => mergeSessionSummaries(current, failed));
      if (isSshLikeProfile(profileForConnect) && isHostKeyFailure(message)) {
        void openHostKeyPrompt(profileForConnect, message, credentials);
      } else {
        setNotice({ title: "连接失败", message });
      }
    }
  }

  async function openHostKeyPrompt(profile: SessionProfile, message: string, credentials?: ConnectionCredentials) {
    setHostKeyPrompt({ profile, message, scan: null, scanError: null, busy: true });
    try {
      const scan = await invokeBackend<HostKeyScanResult>("scan_ssh_host_key", {
        profile: prepareSessionProfile(profile),
        password: credentials?.password ?? null,
        passphrase: credentials?.passphrase ?? null,
      });
      setHostKeyPrompt({ profile, message, scan, scanError: null, busy: false });
    } catch (error) {
      setHostKeyPrompt({ profile, message, scan: null, scanError: formatError(error), busy: false });
    }
  }

  async function applyHostKeyPromptDecision(decision: HostKeyDecisionValue, reconnect: boolean) {
    if (!hostKeyPrompt?.scan) return;
    const profile = prepareSessionProfile(hostKeyPrompt.profile);
    setHostKeyPrompt((current) => current ? { ...current, busy: true } : current);
    try {
      await invokeBackend<TrustedHostKey | null>("trust_scanned_host_key", {
        request: { profile, observation: hostKeyPrompt.scan.observation, decision },
      });
      const nextHostKeys = await callBackend("list_host_keys", {}, hostKeys);
      setHostKeys(nextHostKeys);
      setDraft(profile);
      setHostKeyPrompt(null);
      setNotice({ title: "Host key 已确认", message: reconnect ? "已保存信任决策，正在重新连接。" : "已保存信任决策。" });
      if (reconnect) {
        void connectSession(profile.id);
      }
    } catch (error) {
      setHostKeyPrompt((current) => current ? { ...current, busy: false, scanError: formatError(error) } : current);
    }
  }

  function openHostKeySettingsFromPrompt() {
    if (!hostKeyPrompt) return;
    setDraft(hostKeyPrompt.profile);
    setSessionSettingsSection("验证");
    setDialog("session");
    setHostKeyPrompt(null);
  }

  async function disconnectSession(sessionId = activeId) {
    const session = sessions.find((item) => item.profile.id === sessionId);
    if (!session) return;
    if (isBackendAvailable() && isSshLikeProfile(session.profile) && session.runtime.status !== "connected") {
      return;
    }

    const fallback = setSessionStatus(session, "disconnected");
    const saved = await callBackend("close_session", { sessionId }, fallback);
    const fallbackLog = [...(logs[sessionId] ?? []), createLocalSystemEvent(saved.profile, "PortMate: session disconnected")];
    const nextLog = await callBackend("tail_log", { sessionId, limit: 160 }, fallbackLog);

    setLogs((current) => ({ ...current, [sessionId]: nextLog }));
    setActiveId(sessionId);
    setSessions((current) => {
      const nextSessions = mergeSessionSummaries(current, saved);
      saveLocalSessionSummaries(nextSessions);
      return nextSessions;
    });
  }

  async function routeTerminalInput(sessionId: string, text: string) {
    if (!syncInput) {
      await sendTerminalInput(sessionId, text);
      return;
    }
    const targets = paneSessions
      .filter((session) => session.runtime.status === "connected")
      .map((session) => session.profile.id);
    const uniqueTargets = Array.from(new Set(targets.length ? targets : [sessionId]));
    await Promise.all(uniqueTargets.map((target) => sendTerminalInput(target, text)));
  }

  async function sendTerminalInput(sessionId: string, text: string) {
    if (!sessionId || !text) return;
    const session = sessions.find((item) => item.profile.id === sessionId);
    if (!session) return;

    try {
      if (isBackendAvailable()) {
        await invokeBackend<SessionEvent>("send_text", { sessionId, text });
      } else {
        const event = createLocalSystemEvent(session.profile, text);
        event.direction = "outbound";
        event.stream = "stdout";
        setLogs((current) => ({ ...current, [sessionId]: [...(current[sessionId] ?? []), event] }));
      }
    } catch (error) {
      setLogs((current) => ({
        ...current,
        [sessionId]: [...(current[sessionId] ?? []), createLocalSystemEvent(session.profile, `PortMate: send failed: ${formatError(error)}`)],
      }));
    }
  }

  async function sendTerminalBytes(sessionId: string, bytes: number[]) {
    if (!sessionId || !bytes.length) return;
    const session = sessions.find((item) => item.profile.id === sessionId);
    if (!session) return;

    try {
      if (isBackendAvailable()) {
        await invokeBackend<SessionEvent>("send_bytes", { sessionId, bytes });
      } else {
        const event = createLocalSystemEvent(session.profile, formatHexBytes(bytes));
        event.direction = "outbound";
        event.stream = "stdout";
        setLogs((current) => ({ ...current, [sessionId]: [...(current[sessionId] ?? []), event] }));
      }
    } catch (error) {
      setLogs((current) => ({
        ...current,
        [sessionId]: [...(current[sessionId] ?? []), createLocalSystemEvent(session.profile, `PortMate: send failed: ${formatError(error)}`)],
      }));
    }
  }

  async function runSendPanel() {
    if (sendBusy) return;
    const textPayload = sendMode === "text" ? sendText : "";
    const bytePayload = sendMode === "hex" ? parseHexBytes(sendText) : [];
    if (sendMode === "text" ? !textPayload : !bytePayload.length) return;
    const targets = resolveSendTargets(sendTarget, activeId, sessions, paneSessions);
    if (!targets.length) {
      setNotice({ title: "发送", message: "没有可发送的目标会话。" });
      return;
    }
    setSendBusy(true);
    try {
      for (let index = 0; index < Math.max(1, sendCount); index += 1) {
        await Promise.all(
          targets.map((target) =>
            sendMode === "hex"
              ? sendTerminalBytes(target, bytePayload)
              : sendTerminalInput(target, textPayload),
          ),
        );
        if (index + 1 < Math.max(1, sendCount)) {
          await delay(sendIntervalMs);
        }
      }
      if (sendMode === "text" && textPayload.trim()) {
        setCommandHistory((current) => [textPayload, ...current.filter((item) => item !== textPayload)].slice(0, 200));
      }
    } catch (error) {
      setNotice({ title: "发送失败", message: formatError(error) });
    } finally {
      setSendBusy(false);
    }
  }

  function requestSessionCredentials(profile: SessionProfile): Promise<ConnectionCredentials | null> {
    if (!isSshLikeProfile(profile)) {
      return Promise.resolve({ username: null, password: null, passphrase: null, savePassword: false, savePassphrase: false });
    }

    const ssh = profile.connection;
    const target = describeProfileEndpoint(profile) || profile.name || "SSH";
    const hasPrivateKey = ssh.identityRefs.some((identity) => Boolean(identity.path) || Boolean(identity.secretRef));
    const prompt: CredentialPromptState = {
      target,
      initialUsername: ssh.username || "",
      hasIdentityFiles: hasPrivateKey,
      hasSavedPassword: Boolean(ssh.passwordSecretRef),
      hasSavedPassphrase: Boolean(ssh.passphraseSecretRef),
      needsPassword: ssh.identityPolicy.authOrder.includes("password") || ssh.identityPolicy.authOrder.includes("keyboard-interactive") || !hasPrivateKey,
      authOrder: ssh.identityPolicy.authOrder,
    };

    return new Promise((resolve) => {
      credentialResolverRef.current = resolve;
      setCredentialPrompt(prompt);
    });
  }

  function completeCredentialPrompt(credentials: ConnectionCredentials | null) {
    credentialResolverRef.current?.(credentials);
    credentialResolverRef.current = null;
    setCredentialPrompt(null);
  }

  async function refreshSysmon() {
    if (!active) {
      setNotice({ title: "Sysmon", message: "请先选择一个会话。" });
      return;
    }
    try {
      const snapshot = await invokeBackend<SysmonSnapshot>("refresh_sysmon", { sessionId: active.profile.id });
      setNotice({
        title: "Sysmon",
        message: `CPU ${snapshot.cpuPercent.toFixed(1)}% · Memory ${snapshot.memoryPercent.toFixed(1)}% · RX ${snapshot.rxKbps.toFixed(1)} KiB/s · TX ${snapshot.txKbps.toFixed(1)} KiB/s · Uptime ${snapshot.uptimeSeconds}s`,
      });
      await refreshActiveLog(active.profile.id);
    } catch (error) {
      setNotice({ title: "Sysmon 失败", message: formatError(error) });
    }
  }

  async function setSerialLine(line: "dtr" | "rts", value: boolean) {
    if (!active || active.profile.connection.kind !== "serial") return;
    try {
      const saved = await invokeBackend<SessionSummary>("serial_set_lines", {
        request: { sessionId: active.profile.id, [line]: value },
      });
      setSessions((current) => mergeSessionSummaries(current, saved));
    } catch (error) {
      setNotice({ title: "串口控制失败", message: formatError(error) });
    }
  }

  async function sendSerialBreak() {
    if (!active || active.profile.connection.kind !== "serial") return;
    try {
      await invokeBackend("serial_send_break", { sessionId: active.profile.id });
      await refreshActiveLog(active.profile.id);
    } catch (error) {
      setNotice({ title: "Break 失败", message: formatError(error) });
    }
  }

  return (
    <main className="wind-root" onContextMenu={openAppContextMenu} onClick={() => setContextMenu(null)}>
      <header className="wind-menu">
        <div className="menu-row">
          {menuGroups.map((group) => (
            <div key={group.label} className="menu-item" onMouseLeave={() => setOpenMenu(null)}>
              <button className={openMenu === group.label ? "menu-trigger active" : "menu-trigger"} onClick={() => setOpenMenu(openMenu === group.label ? null : group.label)}>
                {group.label}
              </button>
              {openMenu === group.label && (
                <div className="menu-popover">
                  {group.items.map((item) => (
                    <button
                      key={item}
                      onClick={() => {
                        handleMenuAction(item);
                        setOpenMenu(null);
                      }}
                    >
                      {item}
                    </button>
                  ))}
                </div>
              )}
            </div>
          ))}
        </div>
        <div className="menu-tools">
          <Search size={13} />
          <span>隧道</span>
          <span>专注模式</span>
        </div>
      </header>

      <section className="wind-layout">
        <aside className="left-stack">
          <DockPanel title="资源管理器" accent="#5eead4" actions>
            <FilterLine />
            <TreeList sessions={sessions} groups={sessionGroups} activeId={activeId} onSelect={activateSession} />
          </DockPanel>
          <DockPanel title="文件管理器" accent="#8b5cf6" actions>
            <FileManagerPanel active={active} transfers={transfers} onTransfer={(task) => setTransfers((current) => mergeTransfers(current, task))} onNotice={setNotice} />
          </DockPanel>
        </aside>

        <section className="center-workspace">
          <div className="tab-line">
            {sessions.length ? (
              sessions.map((session) => (
                <button
                  key={session.profile.id}
                  className={session.profile.id === activeId ? "terminal-tab active" : "terminal-tab"}
                  onClick={() => activateSession(session.profile.id)}
                  onContextMenu={(event) => openAppContextMenu(event, session.profile.id)}
                >
                  <span className="tab-mark" style={{ background: tabColors[session.profile.id] ?? "#5eead4" }} />
                  <span className="tab-title">{session.profile.name}</span>
                  <span className={`tab-status ${session.runtime.status}`} />
                  <X
                    size={13}
                    onClick={(event) => {
                      event.stopPropagation();
                      void disconnectSession(session.profile.id);
                    }}
                  />
                </button>
              ))
            ) : (
              <button className="terminal-tab muted" onClick={openNewSessionDialog}>
                <Plus size={13} />
                新建会话
              </button>
            )}
            <button className="terminal-tab new-tab" onClick={openNewSessionDialog}>
              <Plus size={13} />
            </button>
          </div>
          <div className="crumb-line">
            <button onClick={openNewSessionDialog}>
              <Plus size={14} />
            </button>
            <button onClick={() => void refresh()}>
              <RefreshCw size={14} />
            </button>
            {active && active.runtime.status !== "connected" ? (
              <button onClick={() => void connectSession(active.profile.id)}>
                <Play size={14} />
              </button>
            ) : null}
            {active && active.runtime.status === "connected" ? (
              <button onClick={() => void disconnectSession(active.profile.id)}>
                <Square size={13} />
              </button>
            ) : null}
            <span>{active ? `${active.profile.kind} > ${describeEndpoint(active)}` : "未打开会话"}</span>
            {activeSerial && active?.runtime.status === "connected" ? (
              <div className="serial-line-tools">
                <button className={activeSerial.dtr ? "active" : ""} onClick={() => void setSerialLine("dtr", !activeSerial.dtr)}>DTR</button>
                <button className={activeSerial.rts ? "active" : ""} onClick={() => void setSerialLine("rts", !activeSerial.rts)}>RTS</button>
                <button onClick={() => void sendSerialBreak()}>BRK</button>
              </div>
            ) : null}
            {active ? <span className={`runtime-pill ${active.runtime.status}`}>{active.runtime.status}</span> : null}
            {active?.runtime.lastDisconnect ? (
              <span title={active.runtime.lastDisconnectReason ?? undefined}>
                最近断开 {formatEventClock(active.runtime.lastDisconnect)}
                {active.runtime.lastDisconnectReason ? ` · ${active.runtime.lastDisconnectReason}` : ""}
              </span>
            ) : null}
          </div>
          <TerminalPaneGrid
            layout={workspaceLayout}
            panes={paneSessions}
            activeId={activeId}
            eventsBySession={logs}
            blockSelection={blockSelection}
            onInput={(sessionId, text) => void routeTerminalInput(sessionId, text)}
            onActivate={activateSession}
            onClosePane={closeWorkspacePane}
          />
        </section>

        <aside className="right-stack">
          <DockPanel title="会话" accent="#68a7ff" actions>
            <FilterLine />
            <FolderList sessions={sessions} />
          </DockPanel>
          <DockPanel title="历史命令" accent="#f4b860" actions>
            <FilterLine compact />
            <div className="right-tools-list">
              {activeSerial && active ? <SerialMonitorPanel events={logs[active.profile.id] ?? []} /> : null}
              <CommandHistoryPanel history={commandHistory} onPick={setSendText} />
            </div>
          </DockPanel>
        </aside>

        <section className="send-panel">
          <div className="send-tabs">
            <button className="active">发送</button>
            <button>Shell</button>
            <span />
            <Settings size={14} />
            <X size={14} />
          </div>
          <div className="send-toolbar">
            <button className="send-icon-button" onClick={() => void runSendPanel()} disabled={sendBusy}>
              <Play size={14} className="green" />
            </button>
            <Square size={13} />
            <Plus size={13} />
            <span>−</span>
            <X size={13} />
            <label>
              <input type="radio" checked={sendMode === "text"} onChange={() => setSendMode("text")} /> 文本(T)
            </label>
            <label>
              <input type="radio" checked={sendMode === "hex"} onChange={() => setSendMode("hex")} /> Hex(H)
            </label>
            <label>
              计数:
              <input className="number-input" value={sendCount} onChange={(event) => setSendCount(Math.max(1, Number(event.target.value) || 1))} />
            </label>
            <label>
              间隔:
              <input className="number-input" value={sendIntervalMs} onChange={(event) => setSendIntervalMs(Math.max(0, Number(event.target.value) || 0))} />
            </label>
            <label>
              目标:
              <select className="target-input" value={sendTarget} onChange={(event) => setSendTarget(event.target.value as SendTarget)}>
                <option value="active">当前会话</option>
                <option value="panes">打开窗格</option>
                <option value="connected">全部已连接</option>
              </select>
            </label>
            {syncInput ? <span className="sync-badge">同步输入开启</span> : null}
          </div>
          <textarea
            className="send-textarea"
            aria-label="send text"
            value={sendText}
            onChange={(event) => setSendText(event.target.value)}
            onKeyDown={(event) => {
              if (event.ctrlKey && event.key === "Enter" && active) {
                event.preventDefault();
                void runSendPanel();
              }
            }}
          />
        </section>
      </section>

      <footer className="status-bar">
        <span>就绪</span>
        <span />
        <span>{syncInput ? "同步输入" : "远程模式"}</span>
        <span>窗口 -1×-1</span>
        <span>行 1</span>
        <span>字符 0</span>
        <span>{blockSelection ? "块选择" : "Plain Text"}</span>
        <span>{new Date().toLocaleString()}</span>
        <span>PortMate Issues</span>
        <Lock size={12} />
        <span>锁屏</span>
      </footer>

      {contextMenu && (
        <PortMateContextMenu
          state={contextMenu}
          active={contextSession(contextMenu.sessionId)}
          syncInput={syncInput}
          onAction={handleContextMenuAction}
          onColor={(color) => {
            setTabColorFromContext(contextMenu.sessionId, color);
            setContextMenu(null);
          }}
        />
      )}

      {dialog === "terminal" && <TerminalSettingsDialog onClose={() => setDialog(null)} />}
      {dialog === "session" && (
        <SessionSettingsDialog
          draft={draft}
          serialPorts={serialPorts}
          initialSection={sessionSettingsSection}
          onDraftChange={setDraft}
          onSave={saveDraft}
          onSaveAndConnect={saveDraftAndConnect}
          onClose={() => setDialog(null)}
        />
      )}
      {utilityDialog === "transfer" && active && <TransferDialog session={active} transfers={transfers} onClose={() => setUtilityDialog(null)} onTask={(task) => {
        setTransfers((current) => mergeTransfers(current, task));
      }} onNotice={(message) => {
        setNotice({ title: "传输任务", message });
      }} />}
      {utilityDialog === "tunnel" && active && <TunnelDialog session={active} onClose={() => setUtilityDialog(null)} onDone={(label) => {
        setUtilityDialog(null);
        setNotice({ title: "端口转发", message: label });
      }} />}
      {utilityDialog === "tmux" && active && <TmuxDialog session={active} onClose={() => setUtilityDialog(null)} onDone={(message) => {
        setUtilityDialog(null);
        setNotice({ title: "Tmux", message });
        void refreshActiveLog(active.profile.id);
      }} />}
      {utilityDialog === "search" && <SearchDialog state={searchDialog} sessions={sessions} logs={logs} onChange={setSearchDialog} onSelect={(sessionId) => {
        activateSession(sessionId);
        setUtilityDialog(null);
      }} onClose={() => setUtilityDialog(null)} />}
      {utilityDialog === "keys" && <KeyManagerDialog hostKeys={hostKeys} sessions={sessions} onChange={setHostKeys} onProfileChange={applySavedSession} onClose={() => setUtilityDialog(null)} />}
      {utilityDialog === "mcp" && <McpDialog grants={grants} audit={audit} sessions={sessions} onClose={() => setUtilityDialog(null)} onChange={setGrants} />}
      {credentialPrompt && <CredentialDialog request={credentialPrompt} onCancel={() => completeCredentialPrompt(null)} onSubmit={completeCredentialPrompt} />}
      {hostKeyPrompt && (
        <HostKeyConfirmDialog
          state={hostKeyPrompt}
          onDecision={(decision, reconnect) => void applyHostKeyPromptDecision(decision, reconnect)}
          onOpenSettings={openHostKeySettingsFromPrompt}
          onClose={() => setHostKeyPrompt(null)}
        />
      )}
      {notice && <NoticeDialog title={notice.title} message={notice.message} onClose={() => setNotice(null)} />}
    </main>
  );
}

function DockPanel({ title, accent, actions, children }: { title: string; accent: string; actions?: boolean; children: React.ReactNode }) {
  return (
    <section className="dock-panel">
      <header>
        <span className="panel-accent" style={{ background: accent }} />
        <strong>{title}</strong>
        {actions && (
          <div className="panel-actions">
            <Settings size={13} />
            <X size={13} />
          </div>
        )}
      </header>
      {children}
    </section>
  );
}

function FilterLine({ compact = false }: { compact?: boolean }) {
  return <input className={compact ? "filter-line compact" : "filter-line"} placeholder="筛选" />;
}

function TreeList({ sessions, groups, activeId, onSelect }: { sessions: SessionSummary[]; groups: Record<string, SessionSummary[]>; activeId: string; onSelect: (id: string) => void }) {
  if (!sessions.length) return <div className="empty-pane top">没有可用的会话</div>;
  return (
    <div className="tree-list">
      {Object.entries(groups).map(([group, items]) => (
        <div key={group}>
          <div className="tree-folder">
            <ChevronDown size={13} />
            <Folder size={14} />
            {group}
          </div>
          {items.map((session) => (
            <button key={session.profile.id} className={session.profile.id === activeId ? "tree-session active" : "tree-session"} onClick={() => onSelect(session.profile.id)}>
              <span className="cyan-dot" />
              {session.profile.name}
            </button>
          ))}
        </div>
      ))}
    </div>
  );
}

function FolderList({ sessions }: { sessions: SessionSummary[] }) {
  if (!sessions.length) return <div className="empty-pane top">没有可用的会话</div>;

  const grouped = sessions.reduce<Record<string, SessionSummary[]>>((acc, session) => {
    acc[session.profile.kind] ??= [];
    acc[session.profile.kind].push(session);
    return acc;
  }, {});

  return (
    <div className="tree-list">
      {Object.entries(grouped).map(([kind, items]) => (
        <div key={kind}>
          <div className="tree-folder">
            <ChevronRight size={13} />
            <Folder size={14} />
            {kind.toUpperCase()}
          </div>
          {items.map((session) => (
            <div key={session.profile.id} className="tree-session">
              {session.profile.name}
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}

function CommandHistoryPanel({ history, onPick }: { history: string[]; onPick: (value: string) => void }) {
  if (!history.length) return <div className="empty-pane top">没有可用的历史命令</div>;
  return (
    <div className="history-list">
      {history.map((item, index) => (
        <button key={`${index}-${item}`} onClick={() => onPick(item)}>
          <span>{item.replace(/\s+/g, " ").trim() || item}</span>
        </button>
      ))}
    </div>
  );
}

function SerialMonitorPanel({ events }: { events: SessionEvent[] }) {
  const serialEvents = events
    .filter((event) => event.stream !== "audit" && event.text)
    .slice(-24)
    .reverse();
  if (!serialEvents.length) return <div className="empty-pane top">没有串口收发记录</div>;

  return (
    <div className="serial-monitor">
      {serialEvents.map((event) => (
        <div key={event.id} className={`serial-monitor-row ${event.direction}`}>
          <div className="serial-monitor-meta">
            <span>{formatEventClock(event.ts)}</span>
            <strong>{event.direction === "inbound" ? "RX" : event.direction === "outbound" ? "TX" : "SYS"}</strong>
          </div>
          <code>{textToHex(event.text ?? "") || "--"}</code>
          <small>{formatSerialPreview(event.text ?? "")}</small>
        </div>
      ))}
    </div>
  );
}

function TransferList({ transfers, onRetry, onCancel }: { transfers: TransferTask[]; onRetry: (task: TransferTask) => void; onCancel: (task: TransferTask) => void }) {
  if (!transfers.length) return <div className="empty-pane top">没有传输任务</div>;
  return (
    <div className="transfer-list">
      {transfers.slice().reverse().map((task) => (
        <div key={task.id} className="transfer-row">
          <div className="transfer-row-head">
            <strong>{task.protocol}</strong>
            <span>{task.status}</span>
            {task.status === "running" ? (
              <button type="button" onClick={() => onCancel(task)}>取消</button>
            ) : null}
            {task.status === "failed" || task.status === "cancelled" ? (
              <button type="button" onClick={() => onRetry(task)}>重试</button>
            ) : null}
          </div>
          <small>{task.source} → {task.destination}</small>
          <small>
            {formatBytes(task.bytesDone)} / {task.bytesTotal ? formatBytes(task.bytesTotal) : "未知"}
            {task.averageBytesPerSecond ? ` · ${formatBytes(task.averageBytesPerSecond)}/s` : ""}
            {task.startedAt && task.finishedAt ? ` · ${formatDuration(task.startedAt, task.finishedAt)}` : ""}
          </small>
          <div className="transfer-progress">
            <span style={{ width: `${task.bytesTotal ? Math.min(100, (task.bytesDone / task.bytesTotal) * 100) : task.status === "completed" ? 100 : 0}%` }} />
          </div>
        </div>
      ))}
    </div>
  );
}

type FilePanelState = {
  path: string;
  entries: FileEntry[];
  selected: FileEntry | null;
  busy: boolean;
  error: string;
};

type FilePropertiesDialogState = {
  remote: boolean;
  path: string;
  properties: FileProperties | null;
  busy: boolean;
  error: string;
} | null;

type FileDragState = {
  remote: boolean;
  entry: FileEntry;
} | null;

type ExternalDropState = {
  remote: boolean;
  taskIds: string[];
  message: string;
  status: "planning" | "queued" | "completed" | "warning";
} | null;

function FileManagerPanel({
  active,
  transfers,
  onTransfer,
  onNotice,
}: {
  active?: SessionSummary;
  transfers: TransferTask[];
  onTransfer: (task: TransferTask) => void;
  onNotice: (notice: NoticeState) => void;
}) {
  const [localPanel, setLocalPanel] = useState<FilePanelState>(() => ({ path: defaultLocalPath(), entries: [], selected: null, busy: false, error: "" }));
  const [remotePanel, setRemotePanel] = useState<FilePanelState>(() => ({ path: ".", entries: [], selected: null, busy: false, error: "" }));
  const [propertiesDialog, setPropertiesDialog] = useState<FilePropertiesDialogState>(null);
  const [draggedFile, setDraggedFile] = useState<FileDragState>(null);
  const [dropTarget, setDropTarget] = useState<boolean | null>(null);
  const [externalDrop, setExternalDrop] = useState<ExternalDropState>(null);
  const canRemote = Boolean(active && isSshLikeProfile(active.profile) && active.runtime.status === "connected");

  useEffect(() => {
    void loadFiles(false);
  }, []);

  useEffect(() => {
    if (canRemote) {
      void loadFiles(true);
    } else {
      setRemotePanel((current) => ({ ...current, entries: [], selected: null, error: "" }));
    }
  }, [canRemote, active?.profile.id]);

  useEffect(() => {
    setDropTarget(null);
    setExternalDrop(null);
  }, [active?.profile.id]);

  useEffect(() => {
    if (!isBackendAvailable()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void getCurrentWebview().onDragDropEvent((event) => {
      const payload = event.payload;
      if (payload.type === "leave") {
        setDropTarget(null);
        return;
      }
      const remote = filePaneAtPhysicalPosition(payload.position.x, payload.position.y);
      if (!active || remote === null || (remote && !canRemote)) {
        setDropTarget(null);
        return;
      }
      if (payload.type === "drop") {
        setDropTarget(null);
        void startExternalDrop(remote, payload.paths);
      } else {
        setDropTarget(remote);
      }
    }).then((stopListening) => {
      if (disposed) stopListening();
      else unlisten = stopListening;
    }).catch(() => {
      // Native file-drop events are unavailable in browser preview.
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [canRemote, active?.profile.id, localPanel.path, remotePanel.path]);

  useEffect(() => {
    if (!externalDrop || externalDrop.status !== "queued" || !externalDrop.taskIds.length) return;
    const batchTasks = externalDrop.taskIds.map((taskId) => transfers.find((task) => task.id === taskId));
    if (batchTasks.some((task) => !task)) return;
    if (batchTasks.some((task) => task?.status === "queued" || task?.status === "running")) return;
    const failed = batchTasks.filter((task) => task?.status === "failed" || task?.status === "cancelled").length;
    const message = failed
      ? `${batchTasks.length - failed}/${batchTasks.length} 个文件完成，${failed} 个失败或取消`
      : `${batchTasks.length} 个文件传输完成`;
    setExternalDrop((current) => current ? { ...current, message, status: failed ? "warning" : "completed" } : null);
    void loadFiles(externalDrop.remote, externalDrop.remote ? remotePanel.path : localPanel.path);
  }, [externalDrop, transfers]);

  function updatePanel(remote: boolean, patch: Partial<FilePanelState>) {
    const setter = remote ? setRemotePanel : setLocalPanel;
    setter((current) => ({ ...current, ...patch }));
  }

  async function loadFiles(remote: boolean, nextPath = remote ? remotePanel.path : localPanel.path) {
    updatePanel(remote, { busy: true, error: "" });
    try {
      const nextEntries = await invokeBackend<FileEntry[]>("list_files", { request: { sessionId: active?.profile.id ?? null, path: nextPath, remote } });
      updatePanel(remote, { entries: nextEntries, path: nextPath, selected: null });
    } catch (error) {
      updatePanel(remote, { entries: [], error: formatError(error) });
    } finally {
      updatePanel(remote, { busy: false });
    }
  }

  async function createDir(remote: boolean) {
    const panel = remote ? remotePanel : localPanel;
    const name = window.prompt("目录名");
    if (!name?.trim()) return;
    const nextPath = joinFilePath(panel.path, name.trim(), remote);
    try {
      await invokeBackend("create_directory", { request: { sessionId: active?.profile.id ?? null, path: nextPath, remote } });
      await loadFiles(remote, panel.path);
    } catch (error) {
      updatePanel(remote, { error: formatError(error) });
    }
  }

  async function deleteSelected(remote: boolean) {
    const panel = remote ? remotePanel : localPanel;
    if (!panel.selected) return;
    if (!window.confirm(`删除 ${panel.selected.path}?`)) return;
    try {
      await invokeBackend("delete_path", { request: { sessionId: active?.profile.id ?? null, path: panel.selected.path, remote } });
      await loadFiles(remote, panel.path);
    } catch (error) {
      updatePanel(remote, { error: formatError(error) });
    }
  }

  async function renameSelected(remote: boolean) {
    const panel = remote ? remotePanel : localPanel;
    if (!panel.selected) return;
    const nextName = window.prompt("新名称", panel.selected.name);
    if (!nextName?.trim()) return;
    const nextPath = joinFilePath(parentPath(panel.selected.path, remote), nextName.trim(), remote);
    try {
      await invokeBackend("rename_path", { request: { sessionId: active?.profile.id ?? null, oldPath: panel.selected.path, newPath: nextPath, remote } });
      await loadFiles(remote, panel.path);
    } catch (error) {
      updatePanel(remote, { error: formatError(error) });
    }
  }

  async function chmodSelected(remote: boolean) {
    const panel = remote ? remotePanel : localPanel;
    if (!panel.selected) return;
    const modeText = window.prompt("八进制权限", "0644");
    if (!modeText?.trim()) return;
    const mode = Number.parseInt(modeText.replace(/^0o/i, ""), 8);
    if (!Number.isFinite(mode)) return;
    try {
      await invokeBackend("chmod_path", { request: { sessionId: active?.profile.id ?? null, path: panel.selected.path, mode, remote } });
      await loadFiles(remote, panel.path);
    } catch (error) {
      updatePanel(remote, { error: formatError(error) });
    }
  }

  async function showProperties(remote: boolean) {
    const panel = remote ? remotePanel : localPanel;
    if (!panel.selected) return;
    const nextState: NonNullable<FilePropertiesDialogState> = { remote, path: panel.selected.path, properties: null, busy: true, error: "" };
    setPropertiesDialog(nextState);
    try {
      const properties = await invokeBackend<FileProperties>("file_properties", { request: { sessionId: active?.profile.id ?? null, path: panel.selected.path, remote } });
      setPropertiesDialog({ ...nextState, properties, busy: false });
    } catch (error) {
      setPropertiesDialog({ ...nextState, busy: false, error: formatError(error) });
    }
  }

  async function transferBetween(upload: boolean) {
    if (!active || !canRemote) return;
    const selected = upload ? localPanel.selected : remotePanel.selected;
    if (!selected || selected.isDir) return;
    const task = await invokeBackend<TransferTask>("start_transfer", {
      request: {
        sessionId: active.profile.id,
        protocol: "sftp",
        source: upload ? selected.path : `remote:${selected.path}`,
        destination: upload ? `remote:${joinFilePath(remotePanel.path, selected.name, true)}` : joinFilePath(localPanel.path, selected.name, false),
      },
    });
    onTransfer(task);
    onNotice({ title: "传输任务", message: `${task.protocol} ${task.status}: ${task.message ?? ""}` });
  }

  function startFileDrag(remote: boolean, entry: FileEntry, event: ReactDragEvent<HTMLButtonElement>) {
    if (!canRemote || entry.isDir) return;
    setDraggedFile({ remote, entry });
    event.dataTransfer.effectAllowed = "copy";
    event.dataTransfer.setData("application/x-portmate-file", JSON.stringify({ remote, path: entry.path, name: entry.name }));
  }

  function handleDragOver(remote: boolean, event: ReactDragEvent<HTMLElement>) {
    if (!canRemote || !draggedFile || draggedFile.remote === remote || draggedFile.entry.isDir) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
    setDropTarget(remote);
  }

  async function dropFile(remote: boolean, event: ReactDragEvent<HTMLElement>) {
    event.preventDefault();
    const dropped = draggedFile;
    setDropTarget(null);
    setDraggedFile(null);
    if (!active || !canRemote || !dropped || dropped.remote === remote || dropped.entry.isDir) return;
    const targetPanel = remote ? remotePanel : localPanel;
    try {
      const task = await invokeBackend<TransferTask>("start_transfer", {
        request: {
          sessionId: active.profile.id,
          protocol: "sftp",
          source: dropped.remote ? `remote:${dropped.entry.path}` : dropped.entry.path,
          destination: remote ? `remote:${joinFilePath(targetPanel.path, dropped.entry.name, true)}` : joinFilePath(targetPanel.path, dropped.entry.name, false),
        },
      });
      onTransfer(task);
      onNotice({ title: "拖拽传输", message: `${task.protocol} ${task.status}: ${task.message ?? ""}` });
    } catch (error) {
      updatePanel(remote, { error: formatError(error) });
      onNotice({ title: "拖拽传输失败", message: formatError(error) });
    }
  }

  async function startExternalDrop(remote: boolean, paths: string[]) {
    if (!active || (remote && !canRemote) || !paths.length) return;
    const panel = remote ? remotePanel : localPanel;
    setExternalDrop({
      remote,
      taskIds: [],
      message: `正在分析 ${paths.length} 个拖放路径`,
      status: "planning",
    });
    updatePanel(remote, { busy: true, error: "" });
    try {
      const result = await invokeBackend<ExternalDropResult>("start_external_drop", {
        request: {
          sessionId: active.profile.id,
          paths,
          destination: panel.path,
          remote,
        },
      });
      result.tasks.forEach(onTransfer);
      const parts = [
        `${result.tasks.length} 个文件`,
        formatBytes(result.totalBytes),
        `${result.directoriesPrepared} 个目录`,
      ];
      if (result.skipped.length) parts.push(`跳过 ${result.skipped.length} 项`);
      const message = parts.join(" · ");
      setExternalDrop({
        remote,
        taskIds: result.tasks.map((task) => task.id),
        message,
        status: result.tasks.length ? "queued" : result.skipped.length ? "warning" : "completed",
      });
      onNotice({ title: "外部拖放已处理", message });
      if (!result.tasks.length) {
        await loadFiles(remote, panel.path);
      }
    } catch (error) {
      const message = formatError(error);
      setExternalDrop(null);
      updatePanel(remote, { error: message });
      onNotice({ title: "外部拖放失败", message });
    } finally {
      updatePanel(remote, { busy: false });
    }
  }

  async function startPromptTransfer(remote: boolean) {
    if (!active) return;
    const panel = remote ? remotePanel : localPanel;
    const selected = panel.selected;
    if (!selected) return;
    if (remote) {
      const destination = window.prompt("下载到本地路径", selected.name);
      if (!destination) return;
      const task = await invokeBackend<TransferTask>("start_transfer", {
        request: { sessionId: active.profile.id, protocol: "sftp", source: `remote:${selected.path}`, destination },
      });
      onTransfer(task);
      onNotice({ title: "传输任务", message: `${task.protocol} ${task.status}: ${task.message ?? ""}` });
      return;
    }
    const destination = window.prompt("上传到远端路径", `/tmp/${selected.name}`);
    if (!destination) return;
    const task = await invokeBackend<TransferTask>("start_transfer", {
      request: { sessionId: active.profile.id, protocol: "sftp", source: selected.path, destination: `remote:${destination}` },
    });
    onTransfer(task);
    onNotice({ title: "传输任务", message: `${task.protocol} ${task.status}: ${task.message ?? ""}` });
  }

  async function retryTransfer(task: TransferTask) {
    try {
      const retried = await invokeBackend<TransferTask>("retry_transfer", { transferId: task.id });
      onTransfer(retried);
      onNotice({ title: "重试传输", message: `${retried.protocol} ${retried.status}: ${retried.message ?? ""}` });
    } catch (error) {
      onNotice({ title: "重试传输失败", message: formatError(error) });
    }
  }

  async function cancelTransfer(task: TransferTask) {
    try {
      const cancelled = await invokeBackend<TransferTask>("cancel_transfer", { transferId: task.id });
      onTransfer(cancelled);
      onNotice({ title: "取消传输", message: `${cancelled.protocol} ${cancelled.status}: ${cancelled.message ?? ""}` });
    } catch (error) {
      onNotice({ title: "取消传输失败", message: formatError(error) });
    }
  }

  return (
    <div className={canRemote ? "file-manager dual" : "file-manager"}>
      <div className="file-panels">
        <FileBrowserPane
          title="本地"
          remote={false}
          panel={localPanel}
          canTransfer={canRemote}
          transferLabel="上传"
          onPathChange={(path) => setLocalPanel((current) => ({ ...current, path }))}
          onLoad={(path) => void loadFiles(false, path)}
          onSelect={(entry) => setLocalPanel((current) => ({ ...current, selected: entry }))}
          dropActive={dropTarget === false}
          dropStatus={externalDrop?.remote === false ? externalDrop : null}
          onDragStart={(entry, event) => startFileDrag(false, entry, event)}
          onDragEnd={() => {
            setDraggedFile(null);
            setDropTarget(null);
          }}
          onDragOver={(event) => handleDragOver(false, event)}
          onDragLeave={() => setDropTarget((current) => (current === false ? null : current))}
          onDrop={(event) => void dropFile(false, event)}
          onCreateDir={() => void createDir(false)}
          onDelete={() => void deleteSelected(false)}
          onRename={() => void renameSelected(false)}
          onChmod={() => void chmodSelected(false)}
          onProperties={() => void showProperties(false)}
          onTransfer={() => void (canRemote ? transferBetween(true) : startPromptTransfer(false))}
        />
        {canRemote ? (
          <FileBrowserPane
            title="远端"
            remote
            panel={remotePanel}
            canTransfer={canRemote}
            transferLabel="下载"
            onPathChange={(path) => setRemotePanel((current) => ({ ...current, path }))}
            onLoad={(path) => void loadFiles(true, path)}
            onSelect={(entry) => setRemotePanel((current) => ({ ...current, selected: entry }))}
            dropActive={dropTarget === true}
            dropStatus={externalDrop?.remote === true ? externalDrop : null}
            onDragStart={(entry, event) => startFileDrag(true, entry, event)}
            onDragEnd={() => {
              setDraggedFile(null);
              setDropTarget(null);
            }}
            onDragOver={(event) => handleDragOver(true, event)}
            onDragLeave={() => setDropTarget((current) => (current === true ? null : current))}
            onDrop={(event) => void dropFile(true, event)}
            onCreateDir={() => void createDir(true)}
            onDelete={() => void deleteSelected(true)}
            onRename={() => void renameSelected(true)}
            onChmod={() => void chmodSelected(true)}
            onProperties={() => void showProperties(true)}
            onTransfer={() => void transferBetween(false)}
          />
        ) : null}
      </div>
      <TransferList transfers={transfers.slice(-3)} onRetry={(task) => void retryTransfer(task)} onCancel={(task) => void cancelTransfer(task)} />
      {propertiesDialog ? <FilePropertiesDialog state={propertiesDialog} onClose={() => setPropertiesDialog(null)} /> : null}
    </div>
  );
}

function FileBrowserPane({
  title,
  remote,
  panel,
  canTransfer,
  transferLabel,
  dropActive,
  dropStatus,
  onPathChange,
  onLoad,
  onSelect,
  onDragStart,
  onDragEnd,
  onDragOver,
  onDragLeave,
  onDrop,
  onCreateDir,
  onDelete,
  onRename,
  onChmod,
  onProperties,
  onTransfer,
}: {
  title: string;
  remote: boolean;
  panel: FilePanelState;
  canTransfer: boolean;
  transferLabel: string;
  dropActive: boolean;
  dropStatus: ExternalDropState;
  onPathChange: (path: string) => void;
  onLoad: (path: string) => void;
  onSelect: (entry: FileEntry | null) => void;
  onDragStart: (entry: FileEntry, event: ReactDragEvent<HTMLButtonElement>) => void;
  onDragEnd: () => void;
  onDragOver: (event: ReactDragEvent<HTMLElement>) => void;
  onDragLeave: () => void;
  onDrop: (event: ReactDragEvent<HTMLElement>) => void;
  onCreateDir: () => void;
  onDelete: () => void;
  onRename: () => void;
  onChmod: () => void;
  onProperties: () => void;
  onTransfer: () => void;
}) {
  return (
    <section
      className={dropActive ? "file-browser-pane drop-active" : "file-browser-pane"}
      data-file-pane={remote ? "remote" : "local"}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
    >
      <div className="file-toolbar">
        <strong>{title}</strong>
        <input value={panel.path} onChange={(event) => onPathChange(event.target.value)} onKeyDown={(event) => {
          if (event.key === "Enter") {
            onLoad(panel.path);
          }
        }} />
        <button onClick={() => onLoad(panel.path)} disabled={panel.busy}><RefreshCw size={13} /></button>
      </div>
      <div className="file-actions">
        <button onClick={onCreateDir}>新建</button>
        <button onClick={onRename} disabled={!panel.selected}>重命名</button>
        <button onClick={onDelete} disabled={!panel.selected}>删除</button>
        <button onClick={onChmod} disabled={!panel.selected}>权限</button>
        <button onClick={onProperties} disabled={!panel.selected}>属性</button>
        <button onClick={onTransfer} disabled={!panel.selected || panel.selected.isDir || !canTransfer}>{transferLabel}</button>
      </div>
      {panel.error ? (
        <div className="file-error">{panel.error}</div>
      ) : dropStatus ? (
        <div className={`file-pane-status ${dropStatus.status}`}>{dropStatus.message}</div>
      ) : null}
      <div className="file-list">
        <button className="file-row up" onClick={() => onLoad(parentPath(panel.path, remote))}>
          <Folder size={13} />
          <span>..</span>
          <small />
        </button>
        {panel.entries.map((entry) => (
          <button
            key={entry.path}
            className={panel.selected?.path === entry.path ? "file-row active" : "file-row"}
            draggable={canTransfer && !entry.isDir}
            onDragStart={(event) => onDragStart(entry, event)}
            onDragEnd={onDragEnd}
            onClick={() => onSelect(entry)}
            onDoubleClick={() => {
              if (entry.isDir) {
                onLoad(entry.path);
              }
            }}
          >
            {entry.isDir ? <Folder size={13} /> : <File size={13} />}
            <span>{entry.name}</span>
            <small>{entry.isDir ? "dir" : formatBytes(entry.size)}</small>
          </button>
        ))}
      </div>
    </section>
  );
}

function FilePropertiesDialog({ state, onClose }: { state: NonNullable<FilePropertiesDialogState>; onClose: () => void }) {
  const properties = state.properties;
  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <div className="wind-dialog file-properties-dialog">
        <header className="dialog-title">
          <span>文件属性</span>
          <button onClick={onClose}><X size={20} /></button>
        </header>
        <div className="file-properties-content">
          {state.busy ? <div className="empty-pane top">读取中...</div> : null}
          {state.error ? <div className="file-error">{state.error}</div> : null}
          {properties ? (
            <dl className="property-grid">
              <dt>名称</dt>
              <dd>{properties.name}</dd>
              <dt>路径</dt>
              <dd title={properties.path}>{properties.path}</dd>
              <dt>位置</dt>
              <dd>{properties.remote ? "远端" : "本地"}</dd>
              <dt>类型</dt>
              <dd>{formatFileKind(properties)}</dd>
              <dt>大小</dt>
              <dd>{properties.isFile ? `${formatBytes(properties.size)} (${properties.size} B)` : "-"}</dd>
              <dt>权限</dt>
              <dd>{formatFileMode(properties.permissions)}</dd>
              <dt>修改时间</dt>
              <dd>{formatDateTime(properties.modified)}</dd>
              <dt>访问时间</dt>
              <dd>{formatDateTime(properties.accessed)}</dd>
              <dt>创建时间</dt>
              <dd>{formatDateTime(properties.created)}</dd>
            </dl>
          ) : null}
        </div>
        <footer className="utility-actions">
          <button type="button" onClick={onClose}>关闭</button>
        </footer>
      </div>
    </div>
  );
}

function TerminalPaneGrid({
  layout,
  panes,
  activeId,
  eventsBySession,
  blockSelection,
  onInput,
  onActivate,
  onClosePane,
}: {
  layout: WorkspaceLayout;
  panes: SessionSummary[];
  activeId: string;
  eventsBySession: Record<string, SessionEvent[]>;
  blockSelection: boolean;
  onInput: (sessionId: string, text: string) => void;
  onActivate: (sessionId: string) => void;
  onClosePane: (paneIndex: number) => void;
}) {
  if (layout === "single") {
    const active = panes[0];
    return <TerminalCanvas active={active} events={active ? eventsBySession[active.profile.id] ?? [] : []} onInput={onInput} />;
  }

  return (
    <div className={`terminal-pane-grid ${layout} ${blockSelection ? "block-selection" : ""}`}>
      {panes.map((pane, index) => (
        <section key={`${pane.profile.id}-${index}`} className={pane.profile.id === activeId ? "terminal-pane active" : "terminal-pane"} onMouseDown={() => onActivate(pane.profile.id)}>
          <header>
            <strong>{pane.profile.name}</strong>
            <span>{pane.runtime.status}</span>
            <button onClick={(event) => {
              event.stopPropagation();
              onClosePane(index);
            }}>
              <X size={13} />
            </button>
          </header>
          <TerminalCanvas active={pane} events={eventsBySession[pane.profile.id] ?? []} onInput={onInput} />
        </section>
      ))}
      {!panes.length ? <TerminalCanvas events={[]} onInput={onInput} /> : null}
    </div>
  );
}

function TerminalCanvas({ active, events, onInput }: { active?: SessionSummary; events: SessionEvent[]; onInput: (sessionId: string, text: string) => void }) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<XTerm | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const seenEventsRef = useRef<Set<string>>(new Set());
  const pendingInputRef = useRef("");
  const inputFlushTimerRef = useRef<number | null>(null);
  const lastSizeRef = useRef("");
  const lastCopiedSelectionRef = useRef("");

  // Returns whether `id` was already seen. Bounded so a session that stays
  // connected for hours doesn't grow this Set forever; clearing early just
  // risks one harmless re-rendered duplicate line, never data loss.
  function markEventSeen(id: string): boolean {
    if (seenEventsRef.current.size > 4000) {
      seenEventsRef.current.clear();
    }
    if (seenEventsRef.current.has(id)) return true;
    seenEventsRef.current.add(id);
    return false;
  }

  useEffect(() => {
    if (!active || !hostRef.current) return;

    seenEventsRef.current = new Set();
    lastSizeRef.current = "";
    const term = new XTerm({
      cols: active.profile.terminal.cols,
      rows: active.profile.terminal.rows,
      cursorBlink: true,
      convertEol: false,
      drawBoldTextInBrightColors: true,
      fontFamily: active.profile.terminal.fontFamily,
      fontSize: active.profile.terminal.fontSize,
      minimumContrastRatio: 1,
      scrollback: active.profile.terminal.scrollback,
      theme: portmateTerminalTheme,
    });
    const fit = new FitAddon();
    const search = new SearchAddon();
    term.loadAddon(fit);
    term.loadAddon(search);
    term.loadAddon(new WebLinksAddon());
    term.open(hostRef.current);
    term.focus();
    const fitAndReport = () => {
      fit.fit();
      const size = `${term.cols}x${term.rows}`;
      if (lastSizeRef.current !== size) {
        lastSizeRef.current = size;
        if (isBackendAvailable()) {
          void invokeBackend("resize_session", { sessionId: active.profile.id, cols: term.cols, rows: term.rows }).catch(() => {});
        }
      }
    };
    queueMicrotask(fitAndReport);

    const resizeObserver = new ResizeObserver(fitAndReport);
    resizeObserver.observe(hostRef.current);
    const flushInput = () => {
      inputFlushTimerRef.current = null;
      const text = pendingInputRef.current;
      pendingInputRef.current = "";
      if (text) {
        onInput(active.profile.id, text);
      }
    };
    const inputDisposable = term.onData((text) => {
      pendingInputRef.current += text;
      if (inputFlushTimerRef.current === null) {
        inputFlushTimerRef.current = window.setTimeout(flushInput, 12);
      }
    });
    const selectionDisposable = term.onSelectionChange(() => {
      const selected = term.getSelection();
      if (!selected || selected === lastCopiedSelectionRef.current) return;
      lastCopiedSelectionRef.current = selected;
      void navigator.clipboard?.writeText(selected).catch(() => {});
    });
    const pasteFromClipboard = (event: MouseEvent) => {
      event.preventDefault();
      void navigator.clipboard?.readText().then((text) => {
        if (text) onInput(active.profile.id, text);
      }).catch(() => {});
    };
    const pasteOnMiddleClick = (event: MouseEvent) => {
      if (event.button === 1) {
        pasteFromClipboard(event);
      }
    };
    const host = hostRef.current;
    host.addEventListener("auxclick", pasteOnMiddleClick);

    termRef.current = term;
    fitRef.current = fit;

    return () => {
      inputDisposable.dispose();
      selectionDisposable.dispose();
      host.removeEventListener("auxclick", pasteOnMiddleClick);
      if (inputFlushTimerRef.current !== null) {
        window.clearTimeout(inputFlushTimerRef.current);
        inputFlushTimerRef.current = null;
      }
      pendingInputRef.current = "";
      resizeObserver.disconnect();
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
  }, [active?.profile.id]);

  useEffect(() => {
    if (!active || !isBackendAvailable()) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;

    void listen<SessionEvent>("portmate-session-event", (event) => {
      if (disposed || event.payload.sessionId !== active.profile.id) return;
      const term = termRef.current;
      if (!term || markEventSeen(event.payload.id)) return;
      writeTerminalEvent(term, event.payload);
    })
      .then((nextUnlisten) => {
        if (disposed) {
          nextUnlisten();
          return;
        }
        unlisten = nextUnlisten;
      })
      .catch(() => {});

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [active?.profile.id]);

  useEffect(() => {
    const term = termRef.current;
    if (!term) return;
    for (const event of events) {
      if (markEventSeen(event.id)) continue;
      writeTerminalEvent(term, event);
    }
  }, [events, active?.profile.id]);

  return (
    <div className="terminal-canvas">
      {active ? (
        <div ref={hostRef} className="terminal-host" />
      ) : (
        <div className="terminal-empty">未打开会话</div>
      )}
    </div>
  );
}

function PortMateContextMenu({
  state,
  active,
  syncInput,
  onAction,
  onColor,
}: {
  state: NonNullable<ContextMenuState>;
  active?: SessionSummary;
  syncInput: boolean;
  onAction: (action: string, sessionId?: string | null) => void;
  onColor: (color: string) => void;
}) {
  const left = Math.max(8, Math.min(state.x, window.innerWidth - 318));
  const top = Math.max(8, Math.min(state.y, window.innerHeight - 540));
  const sessionId = active?.profile.id ?? state.sessionId;
  const disabled = !active;

  return (
    <div className="portmate-context-menu" style={{ left, top }} onClick={(event) => event.stopPropagation()} onContextMenu={(event) => event.preventDefault()}>
      <ContextSubmenu label="设置标签页颜色(C)" disabled={disabled}>
        <div className="context-color-grid">
          {tabColorChoices.map((color) => (
            <button key={color.value} type="button" onClick={() => onColor(color.value)}>
              <span style={{ background: color.value }} />
              {color.label}
            </button>
          ))}
        </div>
      </ContextSubmenu>
      <ContextSubmenu label="同步输入(S)">
        <ContextMenuButton label={syncInput ? "同步输入已开启" : "开启同步输入"} checked={syncInput} onClick={() => onAction("sync-on", sessionId)} />
        <ContextMenuButton label="关闭同步输入" checked={!syncInput} onClick={() => onAction("sync-off", sessionId)} />
      </ContextSubmenu>
      <ContextMenuButton label="粘贴(P)" shortcut="Ctrl+V" disabled={disabled} onClick={() => onAction("paste", sessionId)} />
      <ContextMenuButton label="重命名视图(R)" disabled={disabled} onClick={() => onAction("rename", sessionId)} />
      <ContextMenuButton label="复制会话(D)" shortcut="Ctrl+Shift+D" disabled={disabled} onClick={() => onAction("duplicate", sessionId)} />
      <ContextMenuButton label="复制SSH通道(D)" disabled />
      <ContextDivider />
      <ContextMenuButton label="复制会话名称(N)" disabled={disabled} onClick={() => onAction("copy-name", sessionId)} />
      <ContextMenuButton label="复制会话 URL(U)" disabled={disabled} onClick={() => onAction("copy-url", sessionId)} />
      <ContextDivider />
      <ContextMenuButton label="重新连接会话(R)" shortcut="Return" disabled={disabled} onClick={() => onAction("reconnect", sessionId)} />
      <ContextMenuButton label="保存会话(S)" shortcut="Ctrl+Shift+S" disabled={disabled} onClick={() => onAction("save", sessionId)} />
      <ContextMenuButton label="水平拆分视图(H)" shortcut="Alt+H" disabled={disabled} onClick={() => onAction("split-h", sessionId)} />
      <ContextMenuButton label="垂直拆分视图(V)" shortcut="Alt+V" disabled={disabled} onClick={() => onAction("split-v", sessionId)} />
      <ContextSubmenu label="拆分为(S)" disabled={disabled}>
        <ContextMenuButton label="水平拆分" onClick={() => onAction("split-h", sessionId)} />
        <ContextMenuButton label="垂直拆分" onClick={() => onAction("split-v", sessionId)} />
      </ContextSubmenu>
      <ContextSubmenu label="移动至分组(M)" disabled={disabled}>
        <ContextMenuButton label="选择分组..." onClick={() => onAction("move-group", sessionId)} />
      </ContextSubmenu>
      <ContextDivider />
      <ContextMenuButton label="关闭视图(C)" shortcut="Ctrl+Shift+W" disabled={disabled} onClick={() => onAction("close", sessionId)} />
      <ContextMenuButton label="关闭所有视图(A)" disabled={!active} onClick={() => onAction("close-all", sessionId)} />
      <ContextMenuButton label="关闭所有非活动视图(A)" disabled={!active} onClick={() => onAction("close-inactive", sessionId)} />
      <ContextMenuButton label="关闭在侧所有视图(R)" disabled={!active} onClick={() => onAction("close-side", sessionId)} />
      <ContextDivider />
      <ContextMenuButton label="会话设置...(S)" disabled={disabled} onClick={() => onAction("settings", sessionId)} />
    </div>
  );
}

function ContextMenuButton({
  label,
  shortcut,
  disabled,
  checked,
  onClick,
}: {
  label: string;
  shortcut?: string;
  disabled?: boolean;
  checked?: boolean;
  onClick?: () => void;
}) {
  return (
    <button type="button" className="context-menu-row" disabled={disabled} onClick={onClick}>
      <span className={checked ? "context-check active" : "context-check"}>{checked ? "✓" : ""}</span>
      <span className="context-label">{label}</span>
      {shortcut ? <span className="context-shortcut">{shortcut}</span> : null}
    </button>
  );
}

function ContextSubmenu({
  label,
  disabled,
  children,
}: {
  label: string;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <div className={disabled ? "context-submenu disabled" : "context-submenu"}>
      <button type="button" className="context-menu-row" disabled={disabled}>
        <span className="context-check" />
        <span className="context-label">{label}</span>
        <span className="context-arrow">›</span>
      </button>
      {!disabled ? <div className="context-submenu-panel">{children}</div> : null}
    </div>
  );
}

function ContextDivider() {
  return <div className="context-divider" />;
}

function TransferDialog({
  session,
  transfers,
  onClose,
  onTask,
  onNotice,
}: {
  session: SessionSummary;
  transfers: TransferTask[];
  onClose: () => void;
  onTask: (task: TransferTask) => void;
  onNotice: (message: string) => void;
}) {
  const [protocol, setProtocol] = useState<TransferTask["protocol"]>("sftp");
  const [source, setSource] = useState("");
  const [destination, setDestination] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const sessionTransfers = transfers.filter((task) => task.sessionId === session.profile.id);
  const runningTransfers = sessionTransfers.filter((task) => task.status === "running");
  const retryableTransfers = sessionTransfers.filter((task) => task.status === "failed" || task.status === "cancelled");

  async function submit(event: FormEvent) {
    event.preventDefault();
    setError("");
    setBusy(true);
    try {
      const task = await invokeBackend<TransferTask>("start_transfer", {
        request: { sessionId: session.profile.id, protocol, source, destination },
      });
      onTask(task);
      onNotice(`${task.protocol} ${task.status}: ${task.message ?? ""}`);
    } catch (error) {
      setError(formatError(error));
    } finally {
      setBusy(false);
    }
  }

  async function retryTransfer(task: TransferTask) {
    try {
      const retried = await invokeBackend<TransferTask>("retry_transfer", { transferId: task.id });
      onTask(retried);
      onNotice(`${retried.protocol} ${retried.status}: ${retried.message ?? ""}`);
    } catch (error) {
      setError(formatError(error));
    }
  }

  async function cancelTransfer(task: TransferTask) {
    try {
      const cancelled = await invokeBackend<TransferTask>("cancel_transfer", { transferId: task.id });
      onTask(cancelled);
      onNotice(`${cancelled.protocol} ${cancelled.status}: ${cancelled.message ?? ""}`);
    } catch (error) {
      setError(formatError(error));
    }
  }

  async function cancelRunningTransfers() {
    for (const task of runningTransfers) {
      await cancelTransfer(task);
    }
  }

  async function retryFailedTransfers() {
    for (const task of retryableTransfers) {
      await retryTransfer(task);
    }
  }

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <form className="wind-dialog utility-dialog transfer-dialog" onSubmit={submit}>
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>传输任务</strong>
          <button type="button" onClick={onClose}><X size={20} /></button>
        </header>
        <section className="utility-content">
          <DialogField label="会话:">
            <input value={session.profile.name} readOnly />
          </DialogField>
          <DialogField label="协议:">
            <select value={protocol} onChange={(event) => setProtocol(event.target.value as TransferTask["protocol"])}>
              <option value="sftp">sftp</option>
              <option value="scp">scp</option>
              <option value="xmodem">xmodem</option>
              <option value="ymodem">ymodem</option>
              <option value="zmodem">zmodem</option>
            </select>
          </DialogField>
          <DialogField label="来源:">
            <input value={source} onChange={(event) => setSource(event.target.value)} placeholder="/local/file 或 remote:/remote/file" />
          </DialogField>
          <DialogField label="目标:">
            <input value={destination} onChange={(event) => setDestination(event.target.value)} placeholder="/local/file 或 remote:/remote/file" />
          </DialogField>
          <div className="transfer-queue-panel">
            <header>
              <strong>队列</strong>
              <div>
                <button type="button" onClick={() => void retryFailedTransfers()} disabled={!retryableTransfers.length}>重试失败</button>
                <button type="button" onClick={() => void cancelRunningTransfers()} disabled={!runningTransfers.length}>取消运行中</button>
              </div>
            </header>
            <TransferList transfers={sessionTransfers} onRetry={(task) => void retryTransfer(task)} onCancel={(task) => void cancelTransfer(task)} />
          </div>
          {error ? <div className="utility-error">{error}</div> : null}
        </section>
        <footer className="utility-actions">
          <button type="button" onClick={onClose}>取消</button>
          <button type="submit" disabled={busy || !source || !destination}>{busy ? "执行中" : "开始"}</button>
        </footer>
      </form>
    </div>
  );
}

function TunnelDialog({ session, onClose, onDone }: { session: SessionSummary; onClose: () => void; onDone: (message: string) => void }) {
  const [mode, setMode] = useState<TunnelSpec["mode"]>("local");
  const [bindHost, setBindHost] = useState("127.0.0.1");
  const [bindPort, setBindPort] = useState("10022");
  const [targetHost, setTargetHost] = useState("127.0.0.1");
  const [targetPort, setTargetPort] = useState("22");
  const [tunnels, setTunnels] = useState<TunnelStatus[]>([]);
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(false);
  const [stoppingId, setStoppingId] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    void refreshTunnels();
    const timer = window.setInterval(() => void refreshTunnels(true), 2000);
    return () => window.clearInterval(timer);
  }, [session.profile.id]);

  async function refreshTunnels(quiet = false) {
    if (!quiet) setLoading(true);
    if (!quiet) setError("");
    try {
      setTunnels(await invokeBackend<TunnelStatus[]>("list_tunnels", { sessionId: session.profile.id }));
    } catch (error) {
      if (!quiet) setError(formatError(error));
    } finally {
      if (!quiet) setLoading(false);
    }
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    setError("");
    setBusy(true);
    try {
      const tunnel = await invokeBackend<TunnelSpec>("create_tunnel", {
        request: {
          sessionId: session.profile.id,
          mode,
          bindHost,
          bindPort: Number(bindPort),
          targetHost: mode === "dynamic" ? "" : targetHost,
          targetPort: mode === "dynamic" ? 0 : Number(targetPort),
        },
      });
      setTunnels((current) => mergeTunnels(current, emptyTunnelStatus(tunnel)));
      onDone(`已创建 ${mode} tunnel：${tunnel.label}`);
    } catch (error) {
      setError(formatError(error));
    } finally {
      setBusy(false);
    }
  }

  async function stopTunnel(tunnel: TunnelStatus) {
    setStoppingId(tunnel.spec.id);
    setError("");
    try {
      await invokeBackend<TunnelStatus>("stop_tunnel", { tunnelId: tunnel.spec.id });
      setTunnels((current) => current.filter((item) => item.spec.id !== tunnel.spec.id));
      onDone(`已停止 tunnel：${tunnel.spec.label}`);
    } catch (error) {
      setError(formatError(error));
    } finally {
      setStoppingId("");
    }
  }

  const sessionTunnels = tunnels.filter((tunnel) => tunnel.spec.enabled);

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <form className="wind-dialog utility-dialog" onSubmit={submit}>
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>端口转发</strong>
          <button type="button" onClick={onClose}><X size={20} /></button>
        </header>
        <section className="utility-content">
          <DialogField label="会话:">
            <input value={session.profile.name} readOnly />
          </DialogField>
          <DialogField label="模式:">
            <select value={mode} onChange={(event) => setMode(event.target.value as TunnelSpec["mode"])}>
              <option value="local">local</option>
              <option value="dynamic">dynamic SOCKS5</option>
              <option value="remote">remote</option>
            </select>
          </DialogField>
          <DialogField label="监听:">
            <input value={bindHost} onChange={(event) => setBindHost(event.target.value)} />
          </DialogField>
          <DialogField label="端口:">
            <input value={bindPort} onChange={(event) => setBindPort(event.target.value)} />
          </DialogField>
          {mode !== "dynamic" ? (
            <>
              <DialogField label="目标:">
                <input value={targetHost} onChange={(event) => setTargetHost(event.target.value)} />
              </DialogField>
              <DialogField label="目标端口:">
                <input value={targetPort} onChange={(event) => setTargetPort(event.target.value)} />
              </DialogField>
            </>
          ) : null}
          <div className="tunnel-panel">
            <header>
              <strong>运行中</strong>
              <button type="button" onClick={() => void refreshTunnels()} disabled={loading} title="刷新 tunnel 列表">
                <RefreshCw size={14} />
              </button>
            </header>
            {sessionTunnels.length ? (
              <div className="tunnel-list">
                {sessionTunnels.map((tunnel) => (
                  <div key={tunnel.spec.id} className={`tunnel-row ${tunnel.lastError ? "degraded" : ""}`}>
                    <div>
                      <strong>{tunnel.spec.label}</strong>
                      <small>{tunnel.spec.mode} · {tunnel.spec.bindHost}:{tunnel.spec.bindPort}{tunnel.spec.mode === "dynamic" ? "" : ` -> ${tunnel.spec.targetHost}:${tunnel.spec.targetPort}`}</small>
                      <small>
                        active {tunnel.activeConnections} · total {tunnel.totalConnections} · TCP→SSH {formatBytes(tunnel.tcpToSshBytes)} · SSH→TCP {formatBytes(tunnel.sshToTcpBytes)}
                      </small>
                      {tunnel.lastError ? <small className="tunnel-error">{tunnel.lastError}</small> : null}
                    </div>
                    <button type="button" onClick={() => void stopTunnel(tunnel)} disabled={stoppingId === tunnel.spec.id} title="停止 tunnel">
                      <Square size={13} />
                    </button>
                  </div>
                ))}
              </div>
            ) : (
              <div className="empty-pane top">{loading ? "正在读取 tunnel" : "没有运行中的 tunnel"}</div>
            )}
          </div>
          {error ? <div className="utility-error">{error}</div> : null}
        </section>
        <footer className="utility-actions">
          <button type="button" onClick={onClose}>取消</button>
          <button type="submit" disabled={busy || !bindHost || !bindPort || (mode !== "dynamic" && (!targetHost || !targetPort))}>{busy ? "创建中" : "创建"}</button>
        </footer>
      </form>
    </div>
  );
}

function TmuxDialog({ session, onClose, onDone }: { session: SessionSummary; onClose: () => void; onDone: (message: string) => void }) {
  const [state, setState] = useState<TmuxState>({ sessions: [], panes: [] });
  const [target, setTarget] = useState("portmate");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    void refreshTmux();
  }, [session.profile.id]);

  async function refreshTmux() {
    setBusy(true);
    setError("");
    try {
      const nextState = await invokeBackend<TmuxState>("list_tmux_state", { sessionId: session.profile.id });
      setState(nextState);
      setTarget((current) => current || nextState.sessions[0]?.name || "portmate");
    } catch (error) {
      setState({ sessions: [], panes: [] });
      setError(formatError(error));
    } finally {
      setBusy(false);
    }
  }

  async function attach(nextTarget = target) {
    const cleanTarget = nextTarget.trim();
    if (!cleanTarget) return;
    setBusy(true);
    setError("");
    try {
      await invokeBackend<SessionEvent>("attach_tmux", { sessionId: session.profile.id, target: cleanTarget });
      onDone(`已发送 tmux attach/new-session：${cleanTarget}`);
    } catch (error) {
      setError(formatError(error));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section className="wind-dialog tmux-dialog">
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>Tmux</strong>
          <button onClick={onClose}><X size={20} /></button>
        </header>
        <div className="tmux-content">
          <div className="tmux-toolbar">
            <input value={target} onChange={(event) => setTarget(event.target.value)} placeholder="session name" />
            <button onClick={() => void attach()} disabled={busy || !target.trim()}><Play size={14} />附着/新建</button>
            <button onClick={() => void refreshTmux()} disabled={busy}><RefreshCw size={14} />刷新</button>
          </div>
          {error ? <div className="utility-error">{error}</div> : null}
          <section className="tmux-section">
            <h2>会话</h2>
            <div className="tmux-list">
              {state.sessions.map((item) => (
                <button key={item.name} onClick={() => {
                  setTarget(item.name);
                  void attach(item.name);
                }}>
                  <strong>{item.name}</strong>
                  <span>{item.windows} windows · {item.attached} attached</span>
                  <small>{item.created ? new Date(item.created).toLocaleString() : "created time unavailable"}</small>
                </button>
              ))}
              {!state.sessions.length ? <div className="empty-pane top">没有检测到 tmux session</div> : null}
            </div>
          </section>
          <section className="tmux-section">
            <h2>窗格</h2>
            <div className="tmux-pane-list">
              {state.panes.map((pane) => (
                <div key={pane.paneId || `${pane.session}-${pane.windowIndex}-${pane.paneIndex}`} className={pane.active ? "active" : ""}>
                  <strong>{pane.session}:{pane.windowIndex}.{pane.paneIndex}</strong>
                  <span>{pane.command || "shell"}</span>
                  <small>{pane.title || pane.paneId}</small>
                </div>
              ))}
              {!state.panes.length ? <div className="empty-pane top">没有可显示的 pane</div> : null}
            </div>
          </section>
        </div>
      </section>
    </div>
  );
}

function SearchDialog({
  state,
  sessions,
  logs,
  onChange,
  onSelect,
  onClose,
}: {
  state: SearchDialogState;
  sessions: SessionSummary[];
  logs: Record<string, SessionEvent[]>;
  onChange: (state: SearchDialogState) => void;
  onSelect: (sessionId: string) => void;
  onClose: () => void;
}) {
  const query = state.query.trim().toLowerCase();
  const results = state.mode === "sessions"
    ? sessions
        .filter((session) => !query || `${session.profile.name} ${describeProfileEndpoint(session.profile)} ${session.profile.group}`.toLowerCase().includes(query))
        .map((session) => ({ id: session.profile.id, title: session.profile.name, detail: describeProfileEndpoint(session.profile) }))
    : Object.values(logs)
        .flat()
        .filter((event) => !query || (event.text ?? "").toLowerCase().includes(query))
        .slice(-80)
        .reverse()
        .map((event) => ({ id: event.sessionId, title: sessions.find((session) => session.profile.id === event.sessionId)?.profile.name ?? event.sessionId, detail: event.text ?? "" }));

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section className="wind-dialog search-dialog">
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>{state.mode === "sessions" ? "会话搜索" : "日志搜索"}</strong>
          <button onClick={onClose}><X size={20} /></button>
        </header>
        <div className="search-content">
          <div className="search-tabs">
            <button className={state.mode === "sessions" ? "active" : ""} onClick={() => onChange({ ...state, mode: "sessions" })}>会话</button>
            <button className={state.mode === "logs" ? "active" : ""} onClick={() => onChange({ ...state, mode: "logs" })}>日志</button>
          </div>
          <input autoFocus value={state.query} onChange={(event) => onChange({ ...state, query: event.target.value })} placeholder="输入关键字" />
          <div className="search-results">
            {results.map((result, index) => (
              <button key={`${result.id}-${index}`} onClick={() => onSelect(result.id)}>
                <strong>{result.title}</strong>
                <span>{result.detail}</span>
              </button>
            ))}
            {!results.length ? <div className="empty-pane top">没有匹配结果</div> : null}
          </div>
        </div>
      </section>
    </div>
  );
}

function KeyManagerDialog({
  hostKeys,
  sessions,
  onChange,
  onProfileChange,
  onClose,
}: {
  hostKeys: HostKeyStore;
  sessions: SessionSummary[];
  onChange: (store: HostKeyStore) => void;
  onProfileChange: (summary: SessionSummary) => void;
  onClose: () => void;
}) {
  const sshSessions = sessions.filter((session) => isSshLikeProfile(session.profile));
  const [profileId, setProfileId] = useState(sshSessions[0]?.profile.id ?? "");
  const [knownHostsText, setKnownHostsText] = useState("");
  const [exportText, setExportText] = useState("");
  const [agentKeys, setAgentKeys] = useState<IdentityRef[]>([]);
  const [clientKeyQuery, setClientKeyQuery] = useState("");
  const [clientKeySourceFilter, setClientKeySourceFilter] = useState<IdentityRef["source"] | "all">("all");
  const [clientKeyProfileFilter, setClientKeyProfileFilter] = useState("all");
  const [clientKeyGroupBy, setClientKeyGroupBy] = useState<ClientIdentityGroupBy>("profile");
  const [selectedClientKeyIds, setSelectedClientKeyIds] = useState<string[]>([]);
  const [selectedAgentKeyIds, setSelectedAgentKeyIds] = useState<string[]>([]);
  const [privateKeyLabel, setPrivateKeyLabel] = useState("profile key");
  const [privateKeyText, setPrivateKeyText] = useState("");
  const [keyScopeFilter, setKeyScopeFilter] = useState<TrustedHostKey["scope"] | "all">("all");
  const [keyProfileFilter, setKeyProfileFilter] = useState("all");
  const [selectedHostKeyIds, setSelectedHostKeyIds] = useState<string[]>([]);
  const [editingKeyId, setEditingKeyId] = useState("");
  const [editDraft, setEditDraft] = useState<HostKeyEditDraft | null>(null);
  const [error, setError] = useState("");
  const [status, setStatus] = useState("");

  const selectedProfile = sshSessions.find((session) => session.profile.id === profileId)?.profile ?? null;
  const editingKey = hostKeys.keys.find((key) => key.id === editingKeyId) ?? null;
  const visibleHostKeys = hostKeys.keys.filter((key) => (
    (keyScopeFilter === "all" || key.scope === keyScopeFilter)
    && (keyProfileFilter === "all" || key.profileId === keyProfileFilter)
  ));
  const selectedVisibleHostKeys = visibleHostKeys.filter((key) => selectedHostKeyIds.includes(key.id));
  const clientIdentityItems = sshSessions.flatMap((session) => {
    const profile = session.profile;
    if (!isSshLikeProfile(profile)) return [];
    return profile.connection.identityRefs.map((identity, index): ClientIdentityItem => ({
      selectionId: clientIdentitySelectionId(profile.id, identity, index),
      profileId: profile.id,
      profileName: profile.name,
      identity,
      jumpInUse: profile.connection.jumps.some((jump) => jump.identityRef === identity.id),
    }));
  });
  const normalizedClientKeyQuery = clientKeyQuery.trim().toLowerCase();
  const visibleClientIdentityItems = clientIdentityItems.filter((item) => (
    (clientKeySourceFilter === "all" || item.identity.source === clientKeySourceFilter)
    && (clientKeyProfileFilter === "all" || item.profileId === clientKeyProfileFilter)
    && (!normalizedClientKeyQuery || `${item.identity.label} ${item.identity.fingerprintSha256 ?? ""} ${item.identity.path ?? ""} ${item.profileName}`.toLowerCase().includes(normalizedClientKeyQuery))
  ));
  const clientIdentityGroups = groupClientIdentityItems(visibleClientIdentityItems, clientKeyGroupBy);
  const selectedClientIdentityItems = clientIdentityItems.filter((item) => selectedClientKeyIds.includes(item.selectionId));
  const selectedAgentKeys = agentKeys.filter((identity) => selectedAgentKeyIds.includes(identityStableKey(identity)));

  useEffect(() => {
    void refreshAgentKeys();
  }, []);

  useEffect(() => {
    if (!sshSessions.some((session) => session.profile.id === profileId)) {
      setProfileId(sshSessions[0]?.profile.id ?? "");
    }
  }, [profileId, sessions]);

  useEffect(() => {
    if (editingKeyId && !hostKeys.keys.some((key) => key.id === editingKeyId)) {
      setEditingKeyId("");
      setEditDraft(null);
    }
    setSelectedHostKeyIds((current) => current.filter((keyId) => hostKeys.keys.some((key) => key.id === keyId)));
  }, [editingKeyId, hostKeys.keys]);

  useEffect(() => {
    const validClientIds = new Set(clientIdentityItems.map((item) => item.selectionId));
    setSelectedClientKeyIds((current) => current.filter((id) => validClientIds.has(id)));
  }, [sessions]);

  useEffect(() => {
    const validAgentIds = new Set(agentKeys.map(identityStableKey));
    setSelectedAgentKeyIds((current) => current.filter((id) => validAgentIds.has(id)));
  }, [agentKeys]);

  async function refreshAgentKeys() {
    if (!isBackendAvailable()) return;
    try {
      setAgentKeys(await invokeBackend<IdentityRef[]>("list_ssh_agent_identities", {}));
    } catch {
      setAgentKeys([]);
    }
  }

  async function importKnownHostsText() {
    if (!profileId || !knownHostsText.trim()) return;
    setError("");
    setStatus("");
    try {
      const nextStore = await invokeBackend<HostKeyStore>("import_known_hosts", {
        request: { profileId, contents: knownHostsText },
      });
      onChange(nextStore);
      setKnownHostsText("");
      setStatus("known_hosts 已导入到选中的 Profile scope");
    } catch (error) {
      setError(formatError(error));
    }
  }

  async function exportKnownHostsText() {
    setError("");
    setStatus("");
    try {
      setExportText(await invokeBackend<string>("export_known_hosts", {}));
    } catch (error) {
      setError(formatError(error));
    }
  }

  async function deleteKey(keyId: string) {
    setError("");
    setStatus("");
    try {
      const nextStore = await invokeBackend<HostKeyStore>("delete_host_key", { keyId });
      onChange(nextStore);
      if (editingKeyId === keyId) {
        setEditingKeyId("");
        setEditDraft(null);
      }
      setStatus("Host key 已删除");
    } catch (error) {
      setError(formatError(error));
    }
  }

  async function deleteSelectedHostKeys() {
    if (!selectedHostKeyIds.length) return;
    setError("");
    setStatus("");
    try {
      const nextStore = await invokeBackend<HostKeyStore>("delete_host_keys", { keyIds: selectedHostKeyIds });
      onChange(nextStore);
      setSelectedHostKeyIds([]);
      setEditingKeyId("");
      setEditDraft(null);
      setStatus(`已删除 ${selectedHostKeyIds.length} 个 host key`);
    } catch (error) {
      setError(formatError(error));
    }
  }

  function toggleHostKeySelection(keyId: string, selected: boolean) {
    setSelectedHostKeyIds((current) => (
      selected
        ? Array.from(new Set([...current, keyId]))
        : current.filter((id) => id !== keyId)
    ));
  }

  function selectVisibleHostKeys() {
    setSelectedHostKeyIds((current) => Array.from(new Set([...current, ...visibleHostKeys.map((key) => key.id)])));
  }

  function startEditKey(key: TrustedHostKey) {
    setEditingKeyId(key.id);
    setEditDraft({
      keyId: key.id,
      profileId: key.profileId ?? profileId,
      alias: key.alias,
      host: key.host,
      port: key.port,
      scope: key.scope,
      label: key.label ?? "",
    });
    setError("");
    setStatus("");
  }

  async function saveEditedHostKey() {
    if (!editDraft) return;
    setError("");
    setStatus("");
    try {
      const nextStore = await invokeBackend<HostKeyStore>("update_host_key", {
        request: {
          keyId: editDraft.keyId,
          profileId: editDraft.profileId || null,
          alias: editDraft.alias,
          host: editDraft.host,
          port: editDraft.port,
          scope: editDraft.scope,
          label: editDraft.label || null,
        },
      });
      onChange(nextStore);
      setEditingKeyId(editDraft.keyId);
      setStatus("Host key 已更新");
    } catch (error) {
      setError(formatError(error));
    }
  }

  async function saveProfileFromManager(profile: SessionProfile, message: string): Promise<boolean> {
    setError("");
    setStatus("");
    try {
      const saved = await invokeBackend<SessionSummary>("save_session_profile", { profile: prepareSessionProfile(profile) });
      onProfileChange(saved);
      setStatus(message);
      return true;
    } catch (error) {
      setError(formatError(error));
      return false;
    }
  }

  async function readPrivateKeyFile(file: File | null) {
    if (!file) return;
    setError("");
    setStatus("");
    try {
      setPrivateKeyText(await file.text());
      if (!privateKeyLabel.trim()) {
        setPrivateKeyLabel(file.name.replace(/\.(pem|key|txt)$/i, "") || "profile key");
      }
      setStatus(`已读取 ${file.name}`);
    } catch (error) {
      setError(formatError(error));
    }
  }

  async function importPrivateKeyToProfile() {
    if (!selectedProfile || !isSshLikeProfile(selectedProfile)) return;
    const privateKey = privateKeyText.trim();
    if (!privateKey) return;
    if (!privateKey.includes("PRIVATE KEY")) {
      setError("私钥内容看起来不是 OpenSSH/PEM private key");
      return;
    }
    setError("");
    setStatus("");
    try {
      const label = privateKeyLabel.trim() || "profile key";
      const response = await invokeBackend<{ secretRef: string }>("save_secret", {
        request: { secretRef: null, secret: privateKeyText },
      });
      const identityRef: IdentityRef = {
        id: `vault:${typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : Date.now()}`,
        label,
        source: "profile-vault",
        fingerprintSha256: null,
        path: null,
        secretRef: response.secretRef,
      };
      const saved = await saveProfileFromManager({
        ...selectedProfile,
        connection: {
          ...selectedProfile.connection,
          identityRefs: [identityRef, ...selectedProfile.connection.identityRefs],
          identityPolicy: {
            ...selectedProfile.connection.identityPolicy,
            identitiesOnly: true,
          },
        },
      }, `已导入私钥到 ${selectedProfile.name}`);
      if (saved) {
        setPrivateKeyText("");
      } else {
        try {
          await invokeBackend("delete_secret", { secretRef: response.secretRef });
        } catch {
          // Preserve the original profile-save error if best-effort cleanup also fails.
        }
      }
    } catch (error) {
      setError(formatError(error));
    }
  }

  async function copyHostKeysToProfile(keys: TrustedHostKey[]) {
    if (!selectedProfile || !isSshLikeProfile(selectedProfile)) return;
    const currentKeys = selectedProfile.connection.trustedHostKeys;
    const copiedKeys: TrustedHostKey[] = [];
    for (const key of keys) {
      const copied: TrustedHostKey = {
        ...key,
        id: `${selectedProfile.id}:${key.alias}:${key.port}:${key.algorithm}:${key.fingerprintSha256}`,
        profileId: selectedProfile.id,
        scope: "profile",
        label: key.label ?? `copied from ${key.scope}`,
        lastSeen: new Date().toISOString(),
      };
      const exists = [...currentKeys, ...copiedKeys].some((item) => (
        item.algorithm === copied.algorithm
        && item.fingerprintSha256 === copied.fingerprintSha256
        && item.alias === copied.alias
        && item.port === copied.port
      ));
      if (!exists) {
        copiedKeys.push(copied);
      }
    }
    if (!copiedKeys.length) {
      setStatus("选中的 Profile 已包含这些 host key");
      return;
    }
    await saveProfileFromManager({
      ...selectedProfile,
      connection: {
        ...selectedProfile.connection,
        trustedHostKeys: [...copiedKeys, ...currentKeys],
      },
    }, `已复制 ${copiedKeys.length} 个 host key 到 ${selectedProfile.name}`);
  }

  async function copyHostKeyToProfile(key: TrustedHostKey) {
    await copyHostKeysToProfile([key]);
  }

  async function copySelectedHostKeysToProfile() {
    await copyHostKeysToProfile(selectedVisibleHostKeys);
  }

  async function copyAgentIdentitiesToProfile(identitiesToCopy: IdentityRef[]) {
    if (!selectedProfile || !isSshLikeProfile(selectedProfile)) return;
    let added = 0;
    let updated = 0;
    let nextIdentities = [...selectedProfile.connection.identityRefs];
    for (const identity of identitiesToCopy) {
      const identityRef: IdentityRef = {
        ...identity,
        id: identity.fingerprintSha256 ? `agent:${identity.fingerprintSha256}` : identity.id,
        source: "agent",
        path: identity.path ?? null,
        secretRef: null,
      };
      const stableKey = identityStableKey(identityRef);
      const existingIndex = nextIdentities.findIndex((item) => identityStableKey(item) === stableKey);
      if (existingIndex >= 0) {
        if (nextIdentities[existingIndex].source !== "agent") continue;
        nextIdentities[existingIndex] = { ...nextIdentities[existingIndex], ...identityRef };
        updated += 1;
      } else {
        nextIdentities = [identityRef, ...nextIdentities];
        added += 1;
      }
    }
    if (!added && !updated) return;
    const saved = await saveProfileFromManager({
      ...selectedProfile,
      connection: {
        ...selectedProfile.connection,
        identityRefs: nextIdentities,
        agentPolicy: {
          ...selectedProfile.connection.agentPolicy,
          enabled: true,
          offerMode: selectedProfile.connection.agentPolicy.offerMode === "disabled" ? "after-profile-keys" : selectedProfile.connection.agentPolicy.offerMode,
        },
      },
    }, `Agent keys: ${added} added, ${updated} updated · ${selectedProfile.name}`);
    if (saved) setSelectedAgentKeyIds([]);
  }

  async function copyAgentIdentityToProfile(identity: IdentityRef) {
    await copyAgentIdentitiesToProfile([identity]);
  }

  async function copyClientIdentitiesToProfile(items: ClientIdentityItem[]) {
    if (!selectedProfile || !isSshLikeProfile(selectedProfile) || !items.length) return;
    const currentIdentities = selectedProfile.connection.identityRefs;
    const nextIdentities = [...currentIdentities];
    let copied = 0;
    let copiedAgent = false;
    let copiedProfileKey = false;
    for (const item of items) {
      const identity = item.identity;
      const stableKey = identityStableKey(identity);
      if (nextIdentities.some((existing) => identityStableKey(existing) === stableKey)) continue;
      let id = identity.id;
      if (nextIdentities.some((existing) => existing.id === id)) {
        id = `${identity.id}:${createLocalId()}`;
      }
      nextIdentities.unshift({ ...identity, id });
      copied += 1;
      copiedAgent ||= identity.source === "agent";
      copiedProfileKey ||= identity.source !== "agent";
    }
    if (!copied) {
      setStatus(`${selectedProfile.name} 已包含选中的 client keys`);
      return;
    }
    const saved = await saveProfileFromManager({
      ...selectedProfile,
      connection: {
        ...selectedProfile.connection,
        identityRefs: nextIdentities,
        identityPolicy: {
          ...selectedProfile.connection.identityPolicy,
          identitiesOnly: copiedProfileKey ? true : selectedProfile.connection.identityPolicy.identitiesOnly,
        },
        agentPolicy: copiedAgent ? {
          ...selectedProfile.connection.agentPolicy,
          enabled: true,
          offerMode: selectedProfile.connection.agentPolicy.offerMode === "disabled" ? "after-profile-keys" : selectedProfile.connection.agentPolicy.offerMode,
        } : selectedProfile.connection.agentPolicy,
      },
    }, `已复制 ${copied} 个 client key 到 ${selectedProfile.name}`);
    if (saved) setSelectedClientKeyIds([]);
  }

  async function moveSelectedClientIdentitiesFirst() {
    if (!selectedClientIdentityItems.length) return;
    setError("");
    setStatus("");
    let updatedProfiles = 0;
    try {
      for (const session of sshSessions) {
        const profile = session.profile;
        if (!isSshLikeProfile(profile)) continue;
        const selectedIds = new Set(selectedClientIdentityItems.map((item) => item.selectionId));
        const selected = profile.connection.identityRefs.filter((identity, index) => (
          selectedIds.has(clientIdentitySelectionId(profile.id, identity, index))
        ));
        if (!selected.length) continue;
        const remaining = profile.connection.identityRefs.filter((identity, index) => (
          !selectedIds.has(clientIdentitySelectionId(profile.id, identity, index))
        ));
        const saved = await invokeBackend<SessionSummary>("save_session_profile", {
          profile: prepareSessionProfile({
            ...profile,
            connection: { ...profile.connection, identityRefs: [...selected, ...remaining] },
          }),
        });
        onProfileChange(saved);
        updatedProfiles += 1;
      }
      setSelectedClientKeyIds([]);
      setStatus(`已在 ${updatedProfiles} 个 Profile 中置顶所选 client keys`);
    } catch (error) {
      setError(formatError(error));
    }
  }

  async function removeSelectedClientIdentities() {
    if (!selectedClientIdentityItems.length) return;
    setError("");
    setStatus("");
    let removed = 0;
    let skipped = 0;
    try {
      for (const session of sshSessions) {
        const profile = session.profile;
        if (!isSshLikeProfile(profile)) continue;
        const selected = selectedClientIdentityItems.filter((item) => item.profileId === profile.id);
        if (!selected.length) continue;
        const removableIds = new Set(selected.filter((item) => !item.jumpInUse).map((item) => item.selectionId));
        skipped += selected.filter((item) => item.jumpInUse).length;
        if (!removableIds.size) continue;
        const saved = await invokeBackend<SessionSummary>("save_session_profile", {
          profile: prepareSessionProfile({
            ...profile,
            connection: {
              ...profile.connection,
              identityRefs: profile.connection.identityRefs.filter((identity, index) => (
                !removableIds.has(clientIdentitySelectionId(profile.id, identity, index))
              )),
            },
          }),
        });
        onProfileChange(saved);
        removed += removableIds.size;
      }
      setSelectedClientKeyIds([]);
      setStatus(`已移除 ${removed} 个 client key 引用${skipped ? `，跳过 ${skipped} 个 Jump Host 使用中的 key` : ""}`);
    } catch (error) {
      setError(formatError(error));
    }
  }

  function toggleClientIdentitySelection(selectionId: string, selected: boolean) {
    setSelectedClientKeyIds((current) => selected
      ? Array.from(new Set([...current, selectionId]))
      : current.filter((id) => id !== selectionId));
  }

  function toggleAgentIdentitySelection(identity: IdentityRef, selected: boolean) {
    const id = identityStableKey(identity);
    setSelectedAgentKeyIds((current) => selected
      ? Array.from(new Set([...current, id]))
      : current.filter((item) => item !== id));
  }

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section className="wind-dialog key-dialog">
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>密钥管理器</strong>
          <button onClick={onClose}><X size={20} /></button>
        </header>
        <div className="key-content">
          <section className="key-list">
            <div className="key-list-toolbar">
              <select value={keyScopeFilter} onChange={(event) => setKeyScopeFilter(event.target.value as TrustedHostKey["scope"] | "all")}>
                <option value="all">全部 scope</option>
                <option value="profile">profile</option>
                <option value="project">project</option>
                <option value="user">user</option>
              </select>
              <select value={keyProfileFilter} onChange={(event) => setKeyProfileFilter(event.target.value)}>
                <option value="all">全部 profile</option>
                {sshSessions.map((session) => (
                  <option key={session.profile.id} value={session.profile.id}>{session.profile.name}</option>
                ))}
              </select>
              <button type="button" onClick={selectVisibleHostKeys} disabled={!visibleHostKeys.length}>全选</button>
              <button type="button" onClick={() => setSelectedHostKeyIds([])} disabled={!selectedHostKeyIds.length}>清除</button>
            </div>
            <div className="key-batch-actions">
              <span>{selectedHostKeyIds.length} selected</span>
              <button type="button" onClick={() => void copySelectedHostKeysToProfile()} disabled={!selectedVisibleHostKeys.length || !selectedProfile}>复制到 Profile</button>
              <button type="button" onClick={() => void deleteSelectedHostKeys()} disabled={!selectedHostKeyIds.length}>删除</button>
            </div>
            {visibleHostKeys.map((key) => (
              <div key={key.id} className="key-row">
                <label className="key-row-select">
                  <input type="checkbox" checked={selectedHostKeyIds.includes(key.id)} onChange={(event) => toggleHostKeySelection(key.id, event.target.checked)} />
                </label>
                <strong>{key.alias}:{key.port}</strong>
                <span>{key.algorithm} · {key.fingerprintSha256}</span>
                <small>{key.scope} · {key.label ?? key.host}</small>
                <div className="key-row-actions">
                  <button onClick={() => startEditKey(key)}>编辑</button>
                  <button onClick={() => void copyHostKeyToProfile(key)} disabled={!selectedProfile}>复制到 Profile</button>
                  <button onClick={() => void deleteKey(key.id)}>删除</button>
                </div>
              </div>
            ))}
            {!hostKeys.keys.length ? <div className="empty-pane top">没有保存的 host key</div> : null}
            {hostKeys.keys.length && !visibleHostKeys.length ? <div className="empty-pane top">当前分组没有 host key</div> : null}
          </section>
          <section className="key-editor">
            <DialogField label="Profile:">
              <select value={profileId} onChange={(event) => setProfileId(event.target.value)}>
                {sshSessions.map((session) => (
                  <option key={session.profile.id} value={session.profile.id}>{session.profile.name}</option>
                ))}
              </select>
            </DialogField>
            {editDraft ? (
              <section className="key-edit-panel">
                <div className="key-edit-heading">
                  <strong>Host Key</strong>
                  <button type="button" onClick={() => { setEditingKeyId(""); setEditDraft(null); }}>关闭</button>
                </div>
                <DialogField label="Alias:">
                  <input value={editDraft.alias} onChange={(event) => setEditDraft({ ...editDraft, alias: event.target.value })} />
                </DialogField>
                <DialogField label="Host:">
                  <input value={editDraft.host} onChange={(event) => setEditDraft({ ...editDraft, host: event.target.value })} />
                </DialogField>
                <DialogField label="Port:">
                  <input type="number" min={1} max={65535} value={editDraft.port} onChange={(event) => setEditDraft({ ...editDraft, port: Number(event.target.value) || 22 })} />
                </DialogField>
                <DialogField label="Scope:">
                  <select value={editDraft.scope} onChange={(event) => setEditDraft({ ...editDraft, scope: event.target.value as TrustedHostKey["scope"] })}>
                    <option value="profile">profile</option>
                    <option value="project">project</option>
                    <option value="user">user</option>
                  </select>
                </DialogField>
                <DialogField label="Profile:">
                  <select value={editDraft.profileId} onChange={(event) => setEditDraft({ ...editDraft, profileId: event.target.value })}>
                    <option value="">无</option>
                    {sshSessions.map((session) => (
                      <option key={session.profile.id} value={session.profile.id}>{session.profile.name}</option>
                    ))}
                  </select>
                </DialogField>
                <DialogField label="Label:">
                  <input value={editDraft.label} onChange={(event) => setEditDraft({ ...editDraft, label: event.target.value })} />
                </DialogField>
                <div className="key-edit-meta">
                  <span>{editingKey?.algorithm ?? ""}</span>
                  <span>{editingKey?.fingerprintSha256 ?? ""}</span>
                </div>
                <div className="key-actions">
                  <button type="button" onClick={() => void saveEditedHostKey()}>保存编辑</button>
                </div>
              </section>
            ) : null}
            <DialogField label="known_hosts:">
              <textarea value={knownHostsText} onChange={(event) => setKnownHostsText(event.target.value)} placeholder="粘贴 OpenSSH known_hosts 内容" />
            </DialogField>
            {error ? <div className="utility-error">{error}</div> : null}
            {status ? <div className="utility-status">{status}</div> : null}
            <div className="key-actions">
              <button onClick={() => void importKnownHostsText()} disabled={!profileId || !knownHostsText.trim()}>导入</button>
              <button onClick={() => void exportKnownHostsText()}>导出</button>
            </div>
            {exportText ? (
              <textarea className="key-export" value={exportText} onChange={(event) => setExportText(event.target.value)} />
            ) : null}
          </section>
          <section className="key-agent-list">
            <div className="key-agent-header">
              <span><KeyRound size={15} /><strong>Client Keys</strong></span>
              <small>{clientIdentityItems.length} identities</small>
            </div>
            <div className="client-key-filters">
              <label className="client-key-search">
                <Search size={14} />
                <input value={clientKeyQuery} onChange={(event) => setClientKeyQuery(event.target.value)} placeholder="搜索 label、指纹或路径" />
              </label>
              <select value={clientKeySourceFilter} onChange={(event) => setClientKeySourceFilter(event.target.value as IdentityRef["source"] | "all")} aria-label="Client key 来源">
                <option value="all">全部来源</option>
                <option value="profile-vault">Profile Vault</option>
                <option value="system-file">System File</option>
                <option value="agent">SSH Agent</option>
                <option value="public-key-only">Public Key</option>
              </select>
              <select value={clientKeyProfileFilter} onChange={(event) => setClientKeyProfileFilter(event.target.value)} aria-label="Client key Profile">
                <option value="all">全部 Profile</option>
                {sshSessions.map((session) => (
                  <option key={session.profile.id} value={session.profile.id}>{session.profile.name}</option>
                ))}
              </select>
              <select value={clientKeyGroupBy} onChange={(event) => setClientKeyGroupBy(event.target.value as ClientIdentityGroupBy)} aria-label="Client key 分组">
                <option value="profile">按 Profile 分组</option>
                <option value="source">按来源分组</option>
              </select>
            </div>
            <div className="client-key-batch">
              <span>{selectedClientIdentityItems.length} selected</span>
              <button type="button" onClick={() => setSelectedClientKeyIds((current) => Array.from(new Set([...current, ...visibleClientIdentityItems.map((item) => item.selectionId)])))} disabled={!visibleClientIdentityItems.length}>全选结果</button>
              <button type="button" onClick={() => setSelectedClientKeyIds([])} disabled={!selectedClientKeyIds.length}>清除</button>
              <div className="client-key-command-group">
                <button className="key-icon-button" type="button" title={`复制到 ${selectedProfile?.name ?? "Profile"}`} aria-label={`复制到 ${selectedProfile?.name ?? "Profile"}`} onClick={() => void copyClientIdentitiesToProfile(selectedClientIdentityItems)} disabled={!selectedClientIdentityItems.length || !selectedProfile}><Copy size={15} /></button>
                <button className="key-icon-button" type="button" title="在各自 Profile 中置顶" aria-label="在各自 Profile 中置顶" onClick={() => void moveSelectedClientIdentitiesFirst()} disabled={!selectedClientIdentityItems.length}><ArrowUp size={15} /></button>
                <button className="key-icon-button danger" type="button" title="从各自 Profile 移除引用" aria-label="从各自 Profile 移除引用" onClick={() => void removeSelectedClientIdentities()} disabled={!selectedClientIdentityItems.length}><Trash2 size={15} /></button>
              </div>
            </div>
            <div className="client-key-groups">
              {clientIdentityGroups.map((group) => (
                <section key={group.id} className="client-key-group">
                  <header><strong>{group.label}</strong><span>{group.items.length}</span></header>
                  {group.items.map((item) => (
                    <label key={item.selectionId} className={`client-key-row${item.jumpInUse ? " in-use" : ""}`}>
                      <input type="checkbox" checked={selectedClientKeyIds.includes(item.selectionId)} onChange={(event) => toggleClientIdentitySelection(item.selectionId, event.target.checked)} />
                      <span className="client-key-main">
                        <strong title={item.identity.label}>{item.identity.label}</strong>
                        <code title={item.identity.fingerprintSha256 ?? item.identity.path ?? item.identity.id}>{item.identity.fingerprintSha256 ?? item.identity.path ?? "No fingerprint"}</code>
                      </span>
                      <span className="client-key-meta">
                        <span>{identitySourceLabel(item.identity.source)}</span>
                        {clientKeyGroupBy === "source" ? <span>{item.profileName}</span> : null}
                        {item.jumpInUse ? <span className="client-key-in-use">Jump Host 使用中</span> : null}
                      </span>
                    </label>
                  ))}
                </section>
              ))}
              {!clientIdentityItems.length ? <div className="empty-pane top">Profile 中还没有 client identity</div> : null}
              {clientIdentityItems.length && !visibleClientIdentityItems.length ? <div className="empty-pane top">当前筛选没有 client identity</div> : null}
            </div>
            <details className="key-import-panel">
              <summary><Plus size={14} />导入私钥到 {selectedProfile?.name ?? "Profile"}</summary>
              <input value={privateKeyLabel} onChange={(event) => setPrivateKeyLabel(event.target.value)} placeholder="Key label" />
              <input type="file" accept=".pem,.key,.txt" onChange={(event) => void readPrivateKeyFile(event.currentTarget.files?.[0] ?? null)} />
              <textarea value={privateKeyText} onChange={(event) => setPrivateKeyText(event.target.value)} placeholder="粘贴 OpenSSH private key" />
              <button onClick={() => void importPrivateKeyToProfile()} disabled={!selectedProfile || !privateKeyText.trim()}>导入到 Profile</button>
            </details>
            <div className="key-agent-header agent-section-header">
              <span><strong>Agent Keys</strong><small>{agentKeys.length} visible</small></span>
              <button onClick={() => void refreshAgentKeys()}>刷新</button>
            </div>
            <div className="client-key-batch agent-key-batch">
              <span>{selectedAgentKeys.length} selected</span>
              <button type="button" onClick={() => setSelectedAgentKeyIds(agentKeys.map(identityStableKey))} disabled={!agentKeys.length}>全选</button>
              <button type="button" onClick={() => setSelectedAgentKeyIds([])} disabled={!selectedAgentKeyIds.length}>清除</button>
              <button className="key-icon-button" type="button" title={`批量添加到 ${selectedProfile?.name ?? "Profile"}`} aria-label={`批量添加到 ${selectedProfile?.name ?? "Profile"}`} onClick={() => void copyAgentIdentitiesToProfile(selectedAgentKeys)} disabled={!selectedAgentKeys.length || !selectedProfile}><UserPlus size={15} /></button>
            </div>
            <div className="agent-key-list">
              {agentKeys.map((identity, index) => (
                <div key={`${identityStableKey(identity)}:${index}`} className="client-key-row agent-row">
                  <input type="checkbox" checked={selectedAgentKeyIds.includes(identityStableKey(identity))} onChange={(event) => toggleAgentIdentitySelection(identity, event.target.checked)} />
                  <span className="client-key-main">
                    <strong title={identity.label}>{identity.label}</strong>
                    <code title={identity.fingerprintSha256 ?? ""}>{identity.fingerprintSha256 ?? "未识别指纹"}</code>
                  </span>
                  <span className="client-key-meta"><span>{identity.path ?? "ssh-agent"}</span></span>
                  <button className="key-icon-button" type="button" title={`添加到 ${selectedProfile?.name ?? "Profile"}`} aria-label={`添加 ${identity.label} 到 ${selectedProfile?.name ?? "Profile"}`} onClick={() => void copyAgentIdentityToProfile(identity)} disabled={!selectedProfile}><UserPlus size={15} /></button>
                </div>
              ))}
              {!agentKeys.length ? <div className="empty-pane top">没有可见的 ssh-agent 身份</div> : null}
            </div>
          </section>
        </div>
      </section>
    </div>
  );
}

const allMcpScopes: McpScope[] = ["read-sessions", "read-logs", "write-input", "transfer", "tunnel", "manage-sessions"];

function McpDialog({
  grants,
  audit,
  sessions,
  onClose,
  onChange,
}: {
  grants: McpGrant[];
  audit: AuditRecord[];
  sessions: SessionSummary[];
  onClose: () => void;
  onChange: (grants: McpGrant[]) => void;
}) {
  const [draft, setDraft] = useState<McpGrant>(() => grants[0] ?? createMcpGrant());
  const [error, setError] = useState("");
  const [httpConfig, setHttpConfig] = useState<McpHttpConfig | null>(null);
  const [httpToken, setHttpToken] = useState("");
  const [httpBusy, setHttpBusy] = useState(false);

  useEffect(() => {
    if (!isBackendAvailable()) return;
    void loadHttpConfig();
  }, []);

  async function loadHttpConfig() {
    try {
      const config = await invokeBackend<McpHttpConfig>("mcp_http_config", {});
      setHttpConfig(config);
    } catch (error) {
      setError(formatError(error));
    }
  }

  async function rotateHttpToken() {
    setError("");
    setHttpBusy(true);
    try {
      const response = await invokeBackend<McpHttpTokenResponse>("rotate_mcp_http_token", {});
      setHttpConfig(response.config);
      setHttpToken(response.token);
    } catch (error) {
      setError(formatError(error));
    } finally {
      setHttpBusy(false);
    }
  }

  async function save() {
    setError("");
    try {
      const saved = await invokeBackend<McpGrant[]>("save_mcp_grant", { grant: draft });
      onChange(saved);
    } catch (error) {
      setError(formatError(error));
    }
  }

  async function revoke(clientId: string) {
    setError("");
    try {
      const saved = await invokeBackend<McpGrant[]>("revoke_mcp_grant", { clientId });
      onChange(saved);
      setDraft(saved[0] ?? createMcpGrant());
    } catch (error) {
      setError(formatError(error));
    }
  }

  function toggleScope(scope: McpScope) {
    setDraft((current) => ({
      ...current,
      scopes: current.scopes.includes(scope) ? current.scopes.filter((item) => item !== scope) : [...current.scopes, scope],
    }));
  }

  function toggleSession(sessionId: string) {
    setDraft((current) => ({
      ...current,
      allowedSessions: current.allowedSessions.includes(sessionId) ? current.allowedSessions.filter((item) => item !== sessionId) : [...current.allowedSessions, sessionId],
    }));
  }

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section className="wind-dialog mcp-dialog">
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>MCP Bridge</strong>
          <button onClick={onClose}><X size={20} /></button>
        </header>
        <div className="mcp-content">
          <aside className="mcp-grants">
            <button className="mcp-new" onClick={() => setDraft(createMcpGrant())}>新建授权</button>
            {grants.map((grant) => (
              <button key={grant.clientId} className={grant.clientId === draft.clientId ? "active" : ""} onClick={() => setDraft(grant)}>
                <strong>{grant.name || grant.clientId}</strong>
                <span>{grant.scopes.join(", ") || "read-only"}</span>
              </button>
            ))}
          </aside>
          <section className="mcp-editor">
            <DialogField label="Client ID:">
              <input value={draft.clientId} onChange={(event) => setDraft({ ...draft, clientId: event.target.value })} />
            </DialogField>
            <DialogField label="名称:">
              <input value={draft.name} onChange={(event) => setDraft({ ...draft, name: event.target.value })} />
            </DialogField>
            <div className="mcp-check-grid">
              {allMcpScopes.map((scope) => (
                <label key={scope}>
                  <input type="checkbox" checked={draft.scopes.includes(scope)} onChange={() => toggleScope(scope)} />
                  {scope}
                </label>
              ))}
            </div>
            <div className="mcp-session-list">
              {sessions.map((session) => (
                <label key={session.profile.id}>
                  <input type="checkbox" checked={draft.allowedSessions.includes(session.profile.id)} onChange={() => toggleSession(session.profile.id)} />
                  {session.profile.name}
                </label>
              ))}
            </div>
            {error ? <div className="utility-error">{error}</div> : null}
            <div className="mcp-actions">
              <button onClick={() => void save()}>保存</button>
              <button onClick={() => void revoke(draft.clientId)} disabled={!draft.clientId}>撤销</button>
            </div>
            <div className="mcp-http-panel">
              <header>
                <strong>HTTP</strong>
                <span>{httpConfig?.tokenAvailable ? "token 已保存" : "未生成 token"}</span>
              </header>
              <div className="mcp-http-row">
                <span>Endpoint</span>
                <code>{httpConfig?.endpoint ?? "http://127.0.0.1:8787/mcp"}</code>
              </div>
              <div className="mcp-http-row">
                <span>Origin</span>
                <code>{httpConfig?.defaultOrigin ?? "http://127.0.0.1:8787"}</code>
              </div>
              <div className="mcp-http-row">
                <span>Token Ref</span>
                <code>{httpConfig?.tokenRef ?? "keychain:mcp-http-token"}</code>
              </div>
              {httpToken ? (
                <div className="mcp-http-token">
                  <span>新 Token</span>
                  <code>{httpToken}</code>
                </div>
              ) : null}
              <textarea readOnly value={httpConfig?.startCommand ?? "PORTMATE_MCP_HTTP=1 cargo run -p portmate-mcp -- --http"} />
              <div className="mcp-actions">
                <button onClick={() => void rotateHttpToken()} disabled={httpBusy}>{httpConfig?.tokenAvailable ? "轮换 Token" : "生成 Token"}</button>
                <button onClick={() => void navigator.clipboard?.writeText(httpConfig?.startCommand ?? "")} disabled={!httpConfig}>复制启动命令</button>
              </div>
            </div>
            <div className="mcp-audit">
              {audit.slice(-8).reverse().map((record) => (
                <div key={record.id}>
                  <strong>{record.action}</strong>
                  <span>{record.actor} · {record.decision}</span>
                </div>
              ))}
            </div>
          </section>
        </div>
      </section>
    </div>
  );
}

function CredentialDialog({
  request,
  onCancel,
  onSubmit,
}: {
  request: CredentialPromptState;
  onCancel: () => void;
  onSubmit: (credentials: ConnectionCredentials) => void;
}) {
  const [username, setUsername] = useState(request.initialUsername);
  const [password, setPassword] = useState("");
  const [passphrase, setPassphrase] = useState("");
  const [savePassword, setSavePassword] = useState(false);
  const [savePassphrase, setSavePassphrase] = useState(false);
  const usernameRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    usernameRef.current?.focus();
    usernameRef.current?.select();
  }, []);

  function submit(event: FormEvent) {
    event.preventDefault();
    const nextUsername = username.trim();
    if (!nextUsername) {
      usernameRef.current?.focus();
      return;
    }
    onSubmit({
      username: nextUsername,
      password: request.needsPassword ? password : null,
      passphrase: request.hasIdentityFiles ? passphrase : null,
      savePassword: request.needsPassword && savePassword,
      savePassphrase: request.hasIdentityFiles && savePassphrase,
    });
  }

  return (
    <div className="dialog-backdrop credential-backdrop" onMouseDown={(event) => {
      if (event.target === event.currentTarget) {
        onCancel();
      }
    }}>
      <form className="wind-dialog credential-dialog" onSubmit={submit}>
        <header className="dialog-title credential-title">
          <span className="app-icon" />
          <div>
            <strong>SSH 连接</strong>
            <small>{request.target}</small>
          </div>
          <button type="button" onClick={onCancel}><X size={20} /></button>
        </header>
        <section className="credential-content">
          <label className="credential-field">
            <span>用户名</span>
            <input ref={usernameRef} value={username} onChange={(event) => setUsername(event.target.value)} autoComplete="username" />
          </label>
          {request.needsPassword ? (
            <label className="credential-field">
              <span>{request.hasSavedPassword ? "登录密码(已存)" : "登录密码"}</span>
              <input value={password} onChange={(event) => setPassword(event.target.value)} type="password" autoComplete="current-password" />
            </label>
          ) : null}
          {request.needsPassword ? (
            <label className="credential-check">
              <input type="checkbox" checked={savePassword} onChange={(event) => setSavePassword(event.target.checked)} disabled={!password} />
              <span>保存登录密码到系统密钥库</span>
            </label>
          ) : null}
          {request.hasIdentityFiles ? (
            <label className="credential-field">
              <span>{request.hasSavedPassphrase ? "私钥口令(已存)" : "私钥口令"}</span>
              <input value={passphrase} onChange={(event) => setPassphrase(event.target.value)} type="password" autoComplete="off" placeholder="没有可留空" />
            </label>
          ) : null}
          {request.hasIdentityFiles ? (
            <label className="credential-check">
              <input type="checkbox" checked={savePassphrase} onChange={(event) => setSavePassphrase(event.target.checked)} disabled={!passphrase} />
              <span>保存私钥口令到系统密钥库</span>
            </label>
          ) : null}
          <div className="credential-meta">
            <span>本次连接</span>
            <span>{request.authOrder.join(" / ")}</span>
          </div>
        </section>
        <footer className="credential-actions">
          <button type="button" onClick={onCancel}>取消</button>
          <button type="submit">连接</button>
        </footer>
      </form>
    </div>
  );
}

function HostKeyConfirmDialog({
  state,
  onDecision,
  onOpenSettings,
  onClose,
}: {
  state: HostKeyPromptState;
  onDecision: (decision: HostKeyDecisionValue, reconnect: boolean) => void;
  onOpenSettings: () => void;
  onClose: () => void;
}) {
  const evaluation = state.scan?.evaluation;
  const observation = state.scan?.observation;
  const fingerprint = evaluation?.status === "mismatch" ? evaluation.observedFingerprintSha256 : evaluation?.status === "unknown" ? evaluation.fingerprintSha256 : evaluation?.status === "trusted" ? evaluation.fingerprintSha256 : "";
  const statusLabel = evaluation?.status === "mismatch" ? "Host key 已变化" : evaluation?.status === "unknown" ? "未知 Host key" : evaluation?.status === "trusted" ? "已信任 Host key" : "正在扫描 Host key";
  const expected = evaluation?.status === "mismatch" ? evaluation.expected.map((key) => key.fingerprintSha256).join(", ") : "";
  const scanLabel = state.scan?.label ?? "目标 SSH";

  return (
    <div className="dialog-backdrop hostkey-backdrop" onMouseDown={(event) => {
      if (event.target === event.currentTarget && !state.busy) {
        onClose();
      }
    }}>
      <section className="wind-dialog hostkey-dialog">
        <header className="dialog-title">
          <span className="app-icon" />
          <div>
            <strong>{statusLabel}</strong>
            <small>{scanLabel} · {describeProfileEndpoint(state.profile)}</small>
          </div>
          <button onClick={onClose} disabled={state.busy}><X size={20} /></button>
        </header>
        <section className="hostkey-content">
          <div className="hostkey-warning">{state.message}</div>
          {state.busy && !state.scan ? <div className="hostkey-row"><span>状态</span><strong>扫描中...</strong></div> : null}
          {observation ? (
            <>
              <div className="hostkey-row"><span>目标</span><strong>{observation.alias ?? observation.host}:{observation.port}</strong></div>
              <div className="hostkey-row"><span>链路</span><strong>{scanLabel}</strong></div>
              <div className="hostkey-row"><span>算法</span><strong>{observation.algorithm}</strong></div>
              <div className="hostkey-fingerprint"><span>SHA-256</span><code>{fingerprint}</code></div>
              {expected ? <div className="hostkey-fingerprint expected"><span>已保存</span><code>{expected}</code></div> : null}
            </>
          ) : null}
          {state.scanError ? <div className="utility-error">{state.scanError}</div> : null}
        </section>
        <footer className="hostkey-actions">
          {state.scan ? (
            <>
              <button type="button" onClick={() => onDecision("trust-once", true)} disabled={state.busy}>仅本次并重连</button>
              <button type="button" onClick={() => onDecision("append-to-profile", true)} disabled={state.busy}>加入 Profile 并重连</button>
              <button type="button" onClick={() => onDecision("append-to-project", true)} disabled={state.busy}>加入 Project 并重连</button>
              <button type="button" onClick={() => onDecision("replace-for-profile", true)} disabled={state.busy}>替换 Profile 并重连</button>
            </>
          ) : (
            <button type="button" onClick={onOpenSettings}>打开验证设置</button>
          )}
          <button type="button" onClick={onClose} disabled={state.busy}>拒绝</button>
        </footer>
      </section>
    </div>
  );
}

function NoticeDialog({ title, message, onClose }: { title: string; message: string; onClose: () => void }) {
  return (
    <div className="dialog-backdrop notice-backdrop" onMouseDown={(event) => {
      if (event.target === event.currentTarget) {
        onClose();
      }
    }}>
      <section className="wind-dialog notice-dialog">
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>{title}</strong>
          <button onClick={onClose}><X size={20} /></button>
        </header>
        <div className="notice-content">{message}</div>
        <footer className="notice-actions">
          <button onClick={onClose}>确定</button>
        </footer>
      </section>
    </div>
  );
}

function writeTerminalEvent(term: XTerm, event: SessionEvent) {
  if (!event.text || event.direction === "outbound") return;
  if (event.direction === "system" || event.stream === "control" || event.stream === "audit") {
    term.writeln(`\x1b[38;5;245m${event.text}\x1b[0m`);
    return;
  }
  term.write(event.text);
}

function TerminalSettingsDialog({ onClose }: { onClose: () => void }) {
  const [activeItem, setActiveItem] = useState("应用");
  const [prefs, setPrefs] = useState<TerminalPrefs>(() => loadLocalValue("portmate.terminalPrefs", createTerminalPrefs()));
  const updatePref = <K extends keyof TerminalPrefs>(key: K, value: TerminalPrefs[K]) => setPrefs((current) => ({ ...current, [key]: value }));
  const showRestartNote = ["应用", "外观", "字体", "扩展"].includes(activeItem);

  function savePrefs() {
    saveLocalValue("portmate.terminalPrefs", prefs);
    onClose();
  }

  return (
    <DialogFrame title="终端设置" className="terminal-settings-dialog" onClose={onClose}>
      <aside className="settings-tree">
        {terminalSettingTree.map((item) => {
          if ("children" in item) {
            return (
              <div key={item.label} className="settings-tree-group">
                <button className={`settings-tree-parent ${activeItem === item.label ? "active" : ""}`} onClick={() => setActiveItem(item.label)}>
                  <ChevronDown size={14} />
                  <span>{item.label}</span>
                </button>
                {item.children.map((child) => (
                  <button key={child} className={`child ${activeItem === child ? "active" : ""}`} onClick={() => setActiveItem(child)}>
                    {child}
                  </button>
                ))}
              </div>
            );
          }

          return (
            <button key={item.label} className={activeItem === item.label ? "active" : ""} onClick={() => setActiveItem(item.label)}>
              {item.label}
            </button>
          );
        })}
      </aside>
      <section className="settings-content">
        <TerminalSettingsContent activeItem={activeItem} prefs={prefs} updatePref={updatePref} />
      </section>
      <div className="dialog-footer">
        <div className="dialog-note">{showRestartNote ? "* 需要重启才能生效" : ""}</div>
        <div className="dialog-actions inline">
          <button onClick={savePrefs}>保存</button>
          <button onClick={onClose}>取消</button>
        </div>
      </div>
    </DialogFrame>
  );
}

function TerminalSettingsContent({
  activeItem,
  prefs,
  updatePref,
}: {
  activeItem: string;
  prefs: TerminalPrefs;
  updatePref: <K extends keyof TerminalPrefs>(key: K, value: TerminalPrefs[K]) => void;
}) {
  switch (activeItem) {
    case "应用":
      return (
        <>
          <SettingsSection title="应用">
            <SettingSelect label="* 语言:(L)" value={prefs.language} options={["Chinese (Simplified) - 中文（简体）", "English", "日本語"]} onChange={(value) => updatePref("language", value)} />
            <SettingSelect label="主题:(T)" value={prefs.theme} options={["dige-black", "portmate-dark", "neutral-dark"]} onChange={(value) => updatePref("theme", value)} />
            <SettingInput label="窗口不透明度:(W)" value={prefs.windowOpacity} onChange={(value) => updatePref("windowOpacity", value)} />
          </SettingsSection>
          <SettingsSection title="启动">
            <SettingRadio label="无会话(N)" checked={prefs.startupMode === "none"} onChange={() => updatePref("startupMode", "none")} name="startup-mode" />
            <SettingRadio label="上次会话(L)" checked={prefs.startupMode === "last"} onChange={() => updatePref("startupMode", "last")} name="startup-mode" />
            <SettingRadio label="指定一个会话或一组会话(S)" checked={prefs.startupMode === "specific"} onChange={() => updatePref("startupMode", "specific")} name="startup-mode" />
            {[0, 1, 2, 3].map((index) => (
              <SettingSelect
                key={index}
                label={`会话 ${index + 1}:`}
                value={prefs.startupSessions[index] ?? ""}
                options={["", "最近使用", "活动工作区", "Serial 默认组", "SSH 默认组"]}
                onChange={(value) => {
                  const next = [...prefs.startupSessions];
                  next[index] = value;
                  updatePref("startupSessions", next);
                }}
              />
            ))}
          </SettingsSection>
          <SettingsSection title="关闭并退出">
            <SettingCheck label="显示关闭标签页确认对话框(T)" checked={prefs.closeTabConfirm} onChange={(value) => updatePref("closeTabConfirm", value)} />
            <SettingCheck label="显示关闭窗口确认对话框(W)" checked={prefs.closeWindowConfirm} onChange={(value) => updatePref("closeWindowConfirm", value)} />
          </SettingsSection>
        </>
      );
    case "外观":
      return (
        <>
          <SettingsSection title="外观">
            <SettingSelect label="强调色:(A)" value={prefs.accent} options={["teal", "blue", "violet", "amber"]} onChange={(value) => updatePref("accent", value)} />
            <SettingSelect label="布局密度:(D)" value={prefs.layoutDensity} options={["紧凑", "标准", "宽松"]} onChange={(value) => updatePref("layoutDensity", value)} />
            <SettingCheck label="紧凑工具栏" checked={prefs.compactChrome} onChange={(value) => updatePref("compactChrome", value)} />
            <SettingCheck label="显示状态栏" checked={prefs.showStatusBar} onChange={(value) => updatePref("showStatusBar", value)} />
          </SettingsSection>
          <SettingsSection title="标签页">
            <SettingSelect label="位置:(P)" value={prefs.tabPosition} options={["顶部", "底部", "左侧"]} onChange={(value) => updatePref("tabPosition", value)} />
            <SettingCheck label="显示会话颜色" checked={prefs.tabShowColors} onChange={(value) => updatePref("tabShowColors", value)} />
            <SettingCheck label="激活标签使用高亮边框" checked={prefs.highlightActiveTab} onChange={(value) => updatePref("highlightActiveTab", value)} />
          </SettingsSection>
        </>
      );
    case "代理":
      return (
        <SettingsSection title="代理">
          <SettingCheck label="启用代理" checked={prefs.proxyEnabled} onChange={(value) => updatePref("proxyEnabled", value)} />
          <SettingSelect label="类型:(T)" value={prefs.proxyType} options={["HTTP", "SOCKS5"]} onChange={(value) => updatePref("proxyType", value)} />
          <SettingInput label="主机:(H)" value={prefs.proxyHost} onChange={(value) => updatePref("proxyHost", value)} />
          <SettingInput label="端口:(P)" value={prefs.proxyPort} onChange={(value) => updatePref("proxyPort", value)} />
          <SettingCheck label="代理 DNS 查询" checked={prefs.proxyDns} onChange={(value) => updatePref("proxyDns", value)} />
        </SettingsSection>
      );
    case "安全":
      return (
        <SettingsSection title="安全">
          <SettingCheck label="空闲后锁屏" checked={prefs.lockOnIdle} onChange={(value) => updatePref("lockOnIdle", value)} />
          <SettingCheck label="粘贴前确认" checked={prefs.confirmPaste} onChange={(value) => updatePref("confirmPaste", value)} />
          <SettingCheck label="日志和 MCP 输出中隐藏敏感字段" checked={prefs.maskSecrets} onChange={(value) => updatePref("maskSecrets", value)} />
          <SettingCheck label="启动时要求主密码" checked={prefs.requireMasterPassword} onChange={(value) => updatePref("requireMasterPassword", value)} />
        </SettingsSection>
      );
    case "标签":
      return (
        <SettingsSection title="标签">
          <SettingSelect label="位置:(P)" value={prefs.tabPosition} options={["顶部", "底部", "左侧"]} onChange={(value) => updatePref("tabPosition", value)} />
          <SettingCheck label="关闭标签前确认" checked={prefs.tabCloseConfirm} onChange={(value) => updatePref("tabCloseConfirm", value)} />
          <SettingCheck label="显示会话颜色" checked={prefs.tabShowColors} onChange={(value) => updatePref("tabShowColors", value)} />
          <SettingCheck label="显示分组颜色条" checked={prefs.groupColorBar} onChange={(value) => updatePref("groupColorBar", value)} />
        </SettingsSection>
      );
    case "终端":
      return (
        <>
          <SettingsSection title="终端">
            <SettingSelect label="默认终端:(T)" value={prefs.defaultTerm} options={["xterm-256color", "xterm", "vt220", "vt420"]} onChange={(value) => updatePref("defaultTerm", value)} />
            <SettingInput label="滚屏行数:(S)" value={prefs.terminalScrollback} onChange={(value) => updatePref("terminalScrollback", Number(value) || 0)} />
            <SettingCheck label="启用 bracketed paste" checked={prefs.pasteBracketed} onChange={(value) => updatePref("pasteBracketed", value)} />
            <SettingCheck label="允许终端鼠标报告" checked={prefs.mouseReporting} onChange={(value) => updatePref("mouseReporting", value)} />
          </SettingsSection>
          <SettingsSection title="输入">
            <SettingCheck label="粘贴前确认" checked={prefs.confirmPaste} onChange={(value) => updatePref("confirmPaste", value)} />
            <SettingCheck label="右键粘贴" checked={prefs.rightClickPaste} onChange={(value) => updatePref("rightClickPaste", value)} />
          </SettingsSection>
        </>
      );
    case "文本":
      return (
        <>
          <SettingsSection title="文本">
            <SettingSelect label="默认视图:(V)" value={prefs.binaryView} options={["Text", "Hex", "Binary"]} onChange={(value) => updatePref("binaryView", value)} />
            <SettingSelect label="插入符:(C)" value={prefs.cursorShape} options={["块", "下划线", "竖线"]} onChange={(value) => updatePref("cursorShape", value)} />
            <SettingCheck label="插入符闪烁" checked={prefs.cursorBlink} onChange={(value) => updatePref("cursorBlink", value)} />
            <SettingCheck label="软换行" checked={prefs.wrapLines} onChange={(value) => updatePref("wrapLines", value)} />
          </SettingsSection>
          <SettingsSection title="字体">
            <SettingSelect label="字体系列 1:" value={prefs.fontFamily1} options={["Roboto Mono", "JetBrains Mono", "Consolas"]} onChange={(value) => updatePref("fontFamily1", value)} />
            <SettingSelect label="字体大小:(S)" value={prefs.fontSize} options={["10 像素", "11 像素", "12 像素", "13 像素"]} onChange={(value) => updatePref("fontSize", value)} />
          </SettingsSection>
        </>
      );
    case "Auto Completion":
      return (
        <>
          <SettingsSection title="完成">
            <SettingCheck label="启用自动补全(A)" checked={prefs.completionEnabled} onChange={(value) => updatePref("completionEnabled", value)} />
            <SettingCheck label="Enable OneKey completion" checked={prefs.oneKeyCompletion} onChange={(value) => updatePref("oneKeyCompletion", value)} />
            <div className="settings-subtitle">自动完成命令使用：</div>
            <SettingCheck label="命令名称(N)" checked={prefs.completionCommandNames} onChange={(value) => updatePref("completionCommandNames", value)} />
            <SettingCheck label="命令选项(O)" checked={prefs.completionCommandOptions} onChange={(value) => updatePref("completionCommandOptions", value)} />
            <SettingCheck label="命令参数(P)" checked={prefs.completionCommandArgs} onChange={(value) => updatePref("completionCommandArgs", value)} />
            <SettingCheck label="历史命令(H)" checked={prefs.completionHistory} onChange={(value) => updatePref("completionHistory", value)} />
            <SettingCheck label="快速命令(Q)" checked={prefs.completionQuickCommands} onChange={(value) => updatePref("completionQuickCommands", value)} />
            <SettingSelect label="输入后开始自动补全:(S)" value={prefs.completionTriggerChars} options={["1 字符", "2 字符", "3 字符"]} onChange={(value) => updatePref("completionTriggerChars", value)} />
          </SettingsSection>
          <SettingsSection title="外观">
            <SettingSelect label="完成列表高度:(H)" value={prefs.completionListHeight} options={["5 行", "7 行", "10 行"]} onChange={(value) => updatePref("completionListHeight", value)} />
            <SettingSelect label="预览最佳匹配项:(P)" value={prefs.completionPreviewMode} options={["无处", "输入框", "列表顶部"]} onChange={(value) => updatePref("completionPreviewMode", value)} />
          </SettingsSection>
        </>
      );
    case "命令历史":
      return (
        <>
          <SettingsSection title="容量">
            <SettingInput label="保留历史天数:(D)" value={prefs.historyRetentionDays} onChange={(value) => updatePref("historyRetentionDays", value)} />
            <SettingInput label="历史大小:(H)" value={prefs.historyLimit} onChange={(value) => updatePref("historyLimit", value)} />
          </SettingsSection>
          <SettingsSection title="存储">
            <SettingCheck label="将命令历史保存到磁盘(S)" checked={prefs.historyEnabled} onChange={(value) => updatePref("historyEnabled", value)} />
            <SettingButtonRow label="已保存的命令历史:">
              <button className="settings-secondary-button" type="button">
                清除(C)
              </button>
            </SettingButtonRow>
          </SettingsSection>
        </>
      );
    case "鼠标追踪":
      return (
        <SettingsSection title="鼠标追踪">
          <SettingCheck label="允许终端应用接收鼠标事件" checked={prefs.mouseReporting} onChange={(value) => updatePref("mouseReporting", value)} />
          <SettingCheck label="选择即复制" checked={prefs.mouseCopyOnSelect} onChange={(value) => updatePref("mouseCopyOnSelect", value)} />
          <SettingCheck label="右键粘贴" checked={prefs.rightClickPaste} onChange={(value) => updatePref("rightClickPaste", value)} />
        </SettingsSection>
      );
    case "二进制":
      return (
        <>
          <SettingsSection title="二进制">
            <SettingInput label="进制数:(B)" value={prefs.binaryRadix} onChange={(value) => updatePref("binaryRadix", value)} />
            <SettingInput label="每行列数:(C)" value={prefs.binaryColumns} onChange={(value) => updatePref("binaryColumns", value)} />
            <SettingInput label="分隔列数:(D)" value={prefs.binaryDividerColumns} onChange={(value) => updatePref("binaryDividerColumns", value)} />
            <SettingInput label="分组字符:(G)" value={prefs.binaryGroupChars} onChange={(value) => updatePref("binaryGroupChars", value)} />
          </SettingsSection>
          <SettingsSection title="">
            <SettingCheck label="交替列背景色(A)" checked={prefs.binaryAlternateRows} onChange={(value) => updatePref("binaryAlternateRows", value)} />
          </SettingsSection>
        </>
      );
    case "插入符":
      return (
        <SettingsSection title="插入符">
          <SettingSelect label="形状:(S)" value={prefs.cursorShape} options={["块", "下划线", "竖线"]} onChange={(value) => updatePref("cursorShape", value)} />
          <SettingSelect label="闪烁:(B)" value={prefs.cursorBlink ? "是" : "否"} options={["否", "是"]} onChange={(value) => updatePref("cursorBlink", value === "是")} />
          <SettingInput label="周期:(P)" value={prefs.cursorBlinkPeriod} onChange={(value) => updatePref("cursorBlinkPeriod", value)} />
          <SettingInput label="宽度:(W)" value={prefs.cursorWidth} onChange={(value) => updatePref("cursorWidth", value)} />
        </SettingsSection>
      );
    case "字体":
      return (
        <>
          <SettingsSection title="默认字体">
            <div className="settings-subtitle">粗体表示固定宽度字体：</div>
            <SettingSelect label="字体系列 1:" value={prefs.fontFamily1} options={["Roboto Mono", "JetBrains Mono", "Consolas"]} onChange={(value) => updatePref("fontFamily1", value)} />
            <SettingSelect label="字体系列 2:" value={prefs.fontFamily2} options={["", "Noto Sans Mono", "Fira Code"]} onChange={(value) => updatePref("fontFamily2", value)} />
            <SettingSelect label="字体系列 3:" value={prefs.fontFamily3} options={["", "Sarasa Mono SC", "Source Code Pro"]} onChange={(value) => updatePref("fontFamily3", value)} />
            <SettingSelect label="字体系列 4:" value={prefs.fontFamily4} options={["", "monospace"]} onChange={(value) => updatePref("fontFamily4", value)} />
            <SettingSelect label="字体粗细:(W)" value={prefs.fontWeight} options={["常规", "中等", "粗体"]} onChange={(value) => updatePref("fontWeight", value)} />
            <SettingSelect label="字体大小:(S)" value={prefs.fontSize} options={["10 像素", "11 像素", "12 像素", "13 像素"]} onChange={(value) => updatePref("fontSize", value)} />
          </SettingsSection>
          <SettingsSection title="">
            <SettingCheck label="首选主题字体(P)" checked={prefs.preferThemeFont} onChange={(value) => updatePref("preferThemeFont", value)} />
          </SettingsSection>
        </>
      );
    case "高亮":
      return (
        <SettingsSection title="高亮">
          <SettingCheck label="高亮插入符所在行(L)" checked={prefs.highlightCursorLine} onChange={(value) => updatePref("highlightCursorLine", value)} />
          <SettingCheck label="高亮插入符(W)" checked={prefs.highlightCursor} onChange={(value) => updatePref("highlightCursor", value)} />
          <SettingCheck label="高亮当前折叠(F)" checked={prefs.highlightFold} onChange={(value) => updatePref("highlightFold", value)} />
          <SettingCheck label="高亮当前配对(P)" checked={prefs.highlightPairs} onChange={(value) => updatePref("highlightPairs", value)} />
          <SettingCheck label="高亮增量搜索(S)" checked={prefs.highlightIncrementalSearch} onChange={(value) => updatePref("highlightIncrementalSearch", value)} />
        </SettingsSection>
      );
    case "换行":
      return (
        <SettingsSection title="换行">
          <SettingCheck label="软换行" checked={prefs.wrapLines} onChange={(value) => updatePref("wrapLines", value)} />
          <SettingSelect label="换行位置:(W)" value={prefs.wrapColumnMode} options={["窗口边界", "80 列", "120 列"]} onChange={(value) => updatePref("wrapColumnMode", value)} />
        </SettingsSection>
      );
    case "小部件":
      return (
        <SettingsSection title="小部件">
          <SettingCheck label="显示侧栏小部件" checked={prefs.fileManagerEnabled || prefs.quickBarEnabled} onChange={(value) => {
            updatePref("fileManagerEnabled", value);
            updatePref("quickBarEnabled", value);
          }} />
        </SettingsSection>
      );
    case "文件管理器":
      return (
        <SettingsSection title="文件管理器">
          <SettingCheck label="启用本地/远端文件管理器" checked={prefs.fileManagerEnabled} onChange={(value) => updatePref("fileManagerEnabled", value)} />
        </SettingsSection>
      );
    case "快捷栏":
      return (
        <SettingsSection title="快捷栏">
          <SettingCheck label="启用快捷命令栏" checked={prefs.quickBarEnabled} onChange={(value) => updatePref("quickBarEnabled", value)} />
        </SettingsSection>
      );
    case "X Server":
      return (
        <SettingsSection title="X Server">
          <SettingCheck label="启用 X11 转发辅助" checked={prefs.xServerEnabled} onChange={(value) => updatePref("xServerEnabled", value)} />
        </SettingsSection>
      );
    case "扩展":
      return (
        <SettingsSection title="扩展">
          <SettingCheck label="启用扩展加载" checked={prefs.extensionsEnabled} onChange={(value) => updatePref("extensionsEnabled", value)} />
          <SettingCheck label="启用 X11 转发辅助" checked={prefs.xServerEnabled} onChange={(value) => updatePref("xServerEnabled", value)} />
        </SettingsSection>
      );
    default:
      return (
        <SettingsSection title={activeItem}>
          <SettingCheck label="启用" checked={prefs.extensionsEnabled} onChange={(value) => updatePref("extensionsEnabled", value)} />
        </SettingsSection>
      );
  }
}

function SessionSettingsDialog({
  draft,
  serialPorts,
  initialSection,
  onDraftChange,
  onSave,
  onSaveAndConnect,
  onClose,
}: {
  draft: SessionProfile;
  serialPorts: string[];
  initialSection: string;
  onDraftChange: (draft: SessionProfile) => void;
  onSave: () => void;
  onSaveAndConnect: () => void;
  onClose: () => void;
}) {
  const [activeProtocol, setActiveProtocol] = useState<ProtocolTab>(() => protocolFromKind(draft.kind));
  const [activeSection, setActiveSection] = useState(initialSection);
  const [prefs, setPrefs] = useState<SessionPrefs>(() => loadLocalValue(`portmate.sessionPrefs.${draft.id}`, loadLocalValue("portmate.sessionPrefs.default", createSessionPrefs())));
  const updatePref = <K extends keyof SessionPrefs>(key: K, value: SessionPrefs[K]) => setPrefs((current) => ({ ...current, [key]: value }));
  const sessionTree = sessionSettingTrees[activeProtocol];
  const allowedSections = useMemo(() => flattenSessionTree(sessionTree), [sessionTree]);

  useEffect(() => {
    if (!allowedSections.includes(activeSection)) {
      setActiveSection("会话");
    }
  }, [activeSection, allowedSections]);

  useEffect(() => {
    setActiveProtocol(protocolFromKind(draft.kind));
    setActiveSection(initialSection);
  }, [draft.id, draft.kind, initialSection]);

  function changeProtocol(tab: ProtocolTab) {
    setActiveProtocol(tab);
    setActiveSection("会话");
    onDraftChange(convertDraftProtocol(draft, tab));
  }

  function saveSessionPrefs() {
    saveLocalValue(`portmate.sessionPrefs.${draft.id}`, prefs);
  }

  function saveDefaultSessionPrefs() {
    saveSessionPrefs();
    saveLocalValue("portmate.sessionPrefs.default", prefs);
  }

  return (
    <DialogFrame title="会话设置" className="session-settings-dialog" onClose={onClose}>
      <div className="protocol-tabs">
        {protocolTabs.map((tab) => (
          <button key={tab} className={tab === activeProtocol ? "active" : ""} onClick={() => changeProtocol(tab)}>
            {tab}
          </button>
        ))}
      </div>
      <aside className="settings-tree session-tree-nav">
        {sessionTree.map((item) => {
          const children = item.children;
          if (children) {
            return (
              <div key={item.label} className="settings-tree-group">
                <button className={`settings-tree-parent ${activeSection === item.label ? "active" : ""}`} onClick={() => setActiveSection(item.label)}>
                  <ChevronDown size={14} />
                  <span>{item.label}</span>
                </button>
                {children.map((child) => (
                  <button key={child} className={`child ${activeSection === child ? "active" : ""}`} onClick={() => setActiveSection(child)}>
                    {child}
                  </button>
                ))}
              </div>
            );
          }

          return (
            <button key={item.label} className={activeSection === item.label ? "active" : ""} onClick={() => setActiveSection(item.label)}>
              {item.label}
            </button>
          );
        })}
      </aside>
      <section className="session-form">
        <SessionSettingsContent activeProtocol={activeProtocol} activeSection={activeSection} draft={draft} serialPorts={serialPorts} onDraftChange={onDraftChange} prefs={prefs} updatePref={updatePref} />
      </section>
      <button className="default-settings" onClick={saveDefaultSessionPrefs}>保存为默认设置...(E)</button>
      <div className="dialog-actions">
        <button onClick={() => {
          saveSessionPrefs();
          onSave();
        }}>保存</button>
        <button onClick={() => {
          saveSessionPrefs();
          onSaveAndConnect();
        }}>保存并连接</button>
        <button onClick={onClose}>取消</button>
      </div>
    </DialogFrame>
  );
}

function SessionSettingsContent({
  activeProtocol,
  activeSection,
  draft,
  serialPorts,
  onDraftChange,
  prefs,
  updatePref,
}: {
  activeProtocol: ProtocolTab;
  activeSection: string;
  draft: SessionProfile;
  serialPorts: string[];
  onDraftChange: (draft: SessionProfile) => void;
  prefs: SessionPrefs;
  updatePref: <K extends keyof SessionPrefs>(key: K, value: SessionPrefs[K]) => void;
}) {
  if (activeSection === "会话") {
    return <SessionConnectionFields activeProtocol={activeProtocol} draft={draft} serialPorts={serialPorts} onDraftChange={onDraftChange} prefs={prefs} updatePref={updatePref} />;
  }

  if (activeSection === "终端") {
    return (
      <>
        <DialogField label="终端:(T)">
          <input value={draft.terminal.term} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, term: event.target.value } })} />
        </DialogField>
        <DialogField label="行:(R)">
          <input type="number" value={draft.terminal.rows} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, rows: Number(event.target.value) } })} />
        </DialogField>
        <DialogField label="列:(C)">
          <input type="number" value={draft.terminal.cols} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, cols: Number(event.target.value) } })} />
        </DialogField>
        <DialogField label="滚屏:(S)">
          <input type="number" value={draft.terminal.scrollback} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, scrollback: Number(event.target.value) } })} />
        </DialogField>
        <DialogField label="字体:(F)">
          <input value={draft.terminal.fontFamily} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, fontFamily: event.target.value } })} />
        </DialogField>
        <DialogField label="字号:(Z)">
          <input type="number" value={draft.terminal.fontSize} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, fontSize: Number(event.target.value) } })} />
        </DialogField>
      </>
    );
  }

  if (activeSection === "Bell") {
    return (
      <>
        <DialogField label="铃声:(B)">
          <select value={prefs.bellMode} onChange={(event) => updatePref("bellMode", event.target.value)}>
            <option>off</option>
            <option>visual</option>
            <option>sound</option>
          </select>
        </DialogField>
        <DialogField label="闪烁:(V)">
          <select value={prefs.visualBell ? "on" : "off"} onChange={(event) => updatePref("visualBell", event.target.value === "on")}>
            <option value="on">开启</option>
            <option value="off">关闭</option>
          </select>
        </DialogField>
      </>
    );
  }

  if (activeSection === "模式") {
    return (
      <>
        <DialogField label="回显:(E)">
          <select value={prefs.localEcho ? "on" : "off"} onChange={(event) => updatePref("localEcho", event.target.value === "on")}>
            <option value="off">由远端控制</option>
            <option value="on">本地回显</option>
          </select>
        </DialogField>
        <DialogField label="同步:(S)">
          <select value={prefs.syncInput ? "on" : "off"} onChange={(event) => updatePref("syncInput", event.target.value === "on")}>
            <option value="off">关闭</option>
            <option value="on">广播输入</option>
          </select>
        </DialogField>
        <DialogField label="焦点:(F)">
          <select value={prefs.focusMode ? "on" : "off"} onChange={(event) => updatePref("focusMode", event.target.value === "on")}>
            <option value="off">普通模式</option>
            <option value="on">隐藏侧栏</option>
          </select>
        </DialogField>
      </>
    );
  }

  if (activeSection === "键盘") {
    return (
      <>
        <DialogField label="退格:(B)">
          <select value={prefs.backspaceMode} onChange={(event) => updatePref("backspaceMode", event.target.value)}>
            <option>DEL</option>
            <option>BS</option>
          </select>
        </DialogField>
        <DialogField label="Alt:(A)">
          <select value={prefs.altSendsEscape ? "escape" : "native"} onChange={(event) => updatePref("altSendsEscape", event.target.value === "escape")}>
            <option value="escape">发送 ESC 前缀</option>
            <option value="native">系统快捷键</option>
          </select>
        </DialogField>
      </>
    );
  }

  if (activeSection === "安全") {
    return (
      <>
        <DialogField label="锁屏:(L)">
          <select value={prefs.focusMode ? "on" : "off"} onChange={(event) => updatePref("focusMode", event.target.value === "on")}>
            <option value="off">关闭</option>
            <option value="on">空闲后锁屏</option>
          </select>
        </DialogField>
        <DialogField label="粘贴:(P)">
          <select value={prefs.confirmPaste ? "confirm" : "direct"} onChange={(event) => updatePref("confirmPaste", event.target.value === "confirm")}>
            <option value="confirm">确认后粘贴</option>
            <option value="direct">直接粘贴</option>
          </select>
        </DialogField>
        <DialogField label="记录:(R)">
          <select value={draft.logging.redactSecrets ? "redact" : "plain"} onChange={(event) => onDraftChange({ ...draft, logging: { ...draft.logging, redactSecrets: event.target.value === "redact" } })}>
            <option value="redact">隐藏敏感字段</option>
            <option value="plain">完整记录</option>
          </select>
        </DialogField>
      </>
    );
  }

  if (activeSection === "日志") {
    return (
      <>
        <DialogField label="启用:(E)">
          <select value={draft.logging.enabled ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, logging: { ...draft.logging, enabled: event.target.value === "on" } })}>
            <option value="on">开启</option>
            <option value="off">关闭</option>
          </select>
        </DialogField>
        <DialogField label="Raw:(R)">
          <select value={draft.logging.raw ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, logging: { ...draft.logging, raw: event.target.value === "on" } })}>
            <option value="on">开启</option>
            <option value="off">关闭</option>
          </select>
        </DialogField>
        <DialogField label="Text:(T)">
          <select value={draft.logging.text ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, logging: { ...draft.logging, text: event.target.value === "on" } })}>
            <option value="on">开启</option>
            <option value="off">关闭</option>
          </select>
        </DialogField>
        <DialogField label="JSONL:(J)">
          <select value={draft.logging.jsonl ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, logging: { ...draft.logging, jsonl: event.target.value === "on" } })}>
            <option value="on">开启</option>
            <option value="off">关闭</option>
          </select>
        </DialogField>
        <DialogField label="路径:(P)">
          <input value={draft.logging.pathTemplate} onChange={(event) => onDraftChange({ ...draft, logging: { ...draft.logging, pathTemplate: event.target.value } })} />
        </DialogField>
      </>
    );
  }

  if (activeSection === "窗口") {
    return (
      <>
        <DialogField label="拆分:(S)">
          <select value={prefs.splitMode} onChange={(event) => updatePref("splitMode", event.target.value)}>
            <option>none</option>
            <option>vertical</option>
            <option>horizontal</option>
            <option>grid</option>
          </select>
        </DialogField>
        <DialogField label="标签色:(C)">
          <input value={prefs.tabColor} onChange={(event) => updatePref("tabColor", event.target.value)} />
        </DialogField>
      </>
    );
  }

  if (activeSection === "选择") {
    return (
      <>
        <DialogField label="复制:(C)">
          <select value={prefs.copyOnSelect ? "select" : "manual"} onChange={(event) => updatePref("copyOnSelect", event.target.value === "select")}>
            <option value="select">选择即复制</option>
            <option value="manual">手动复制</option>
          </select>
        </DialogField>
        <DialogField label="右键:(R)">
          <select value={prefs.rightClickPaste ? "paste" : "menu"} onChange={(event) => updatePref("rightClickPaste", event.target.value === "paste")}>
            <option value="paste">粘贴</option>
            <option value="menu">菜单</option>
          </select>
        </DialogField>
      </>
    );
  }

  if (activeSection === "自动化" || activeSection === "触发器") {
    return <TriggerFields draft={draft} onDraftChange={onDraftChange} />;
  }

  if (activeProtocol === "Shell" && activeSection === "进程") {
    return <ShellProcessFields draft={draft} onDraftChange={onDraftChange} prefs={prefs} updatePref={updatePref} />;
  }

  if (activeProtocol === "Shell" && activeSection === "Shell") {
    return <ShellProcessFields draft={draft} onDraftChange={onDraftChange} prefs={prefs} updatePref={updatePref} />;
  }

  if ((activeProtocol === "SSH" || activeProtocol === "Tmux") && (activeSection === "SSH" || activeSection === "Tmux")) {
    return <SshAdvancedFields section="连接" draft={draft} onDraftChange={onDraftChange} prefs={prefs} updatePref={updatePref} />;
  }

  if ((activeProtocol === "SSH" || activeProtocol === "Tmux") && ["连接", "代理", "验证", "代理人", "密码", "密钥交换", "MAC 哈希", "公钥", "SFTP", "X11"].includes(activeSection)) {
    return <SshAdvancedFields section={activeSection} draft={draft} onDraftChange={onDraftChange} prefs={prefs} updatePref={updatePref} />;
  }

  if (activeProtocol === "Telnet" && activeSection === "Telnet") {
    return <TcpLikeAdvancedFields protocol="Telnet" section="连接" draft={draft} onDraftChange={onDraftChange} prefs={prefs} updatePref={updatePref} />;
  }

  if (activeProtocol === "Tcp" && activeSection === "Tcp") {
    return <TcpLikeAdvancedFields protocol="Tcp" section="连接" draft={draft} onDraftChange={onDraftChange} prefs={prefs} updatePref={updatePref} />;
  }

  if ((activeProtocol === "Telnet" || activeProtocol === "Tcp") && ["连接", "代理"].includes(activeSection)) {
    return <TcpLikeAdvancedFields protocol={activeProtocol} section={activeSection} draft={draft} onDraftChange={onDraftChange} prefs={prefs} updatePref={updatePref} />;
  }

  if (activeProtocol === "Serial" && activeSection === "串口") {
    return <SerialAdvancedFields draft={draft} onDraftChange={onDraftChange} prefs={prefs} updatePref={updatePref} />;
  }

  if (activeProtocol === "Serial" && activeSection === "协议") {
    return <SerialAdvancedFields draft={draft} onDraftChange={onDraftChange} prefs={prefs} updatePref={updatePref} />;
  }

  if (activeSection === "X/Y/Z Modem") {
    return (
      <>
        <DialogField label="XModem:">
          <select value={draft.transfer.xmodem ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, transfer: { ...draft.transfer, xmodem: event.target.value === "on" } })}>
            <option value="on">开启</option>
            <option value="off">关闭</option>
          </select>
        </DialogField>
        <DialogField label="YModem:">
          <select value={draft.transfer.ymodem ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, transfer: { ...draft.transfer, ymodem: event.target.value === "on" } })}>
            <option value="on">开启</option>
            <option value="off">关闭</option>
          </select>
        </DialogField>
        <DialogField label="ZModem:">
          <select value={draft.transfer.zmodem ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, transfer: { ...draft.transfer, zmodem: event.target.value === "on" } })}>
            <option value="on">开启</option>
            <option value="off">关闭</option>
          </select>
        </DialogField>
        <DialogField label="限速 B/s:">
          <input type="number" min={0} value={draft.transfer.rateLimitBytesPerSecond ?? 0} onChange={(event) => onDraftChange({ ...draft, transfer: { ...draft.transfer, rateLimitBytesPerSecond: Number(event.target.value) > 0 ? Number(event.target.value) : null } })} />
        </DialogField>
        <DialogField label="目录:(D)">
          <input value={draft.transfer.defaultLocalDir ?? ""} onChange={(event) => onDraftChange({ ...draft, transfer: { ...draft.transfer, defaultLocalDir: event.target.value || null } })} />
        </DialogField>
      </>
    );
  }

  return null;
}

function SessionConnectionFields({
  activeProtocol,
  draft,
  serialPorts,
  onDraftChange,
  prefs,
  updatePref,
}: {
  activeProtocol: ProtocolTab;
  draft: SessionProfile;
  serialPorts: string[];
  onDraftChange: (draft: SessionProfile) => void;
  prefs: SessionPrefs;
  updatePref: <K extends keyof SessionPrefs>(key: K, value: SessionPrefs[K]) => void;
}) {
  if (activeProtocol === "Shell") {
    const shell = draft.connection.kind === "shell" ? draft.connection : createShellConnection();
    return (
      <>
        <DialogField label="Shell:(S)">
          <select
            value={prefs.shellPreset}
            onChange={(event) => {
              updatePref("shellPreset", event.target.value);
              onDraftChange({ ...draft, kind: "shell", connection: { ...shell, program: event.target.value } });
            }}
          >
            <option>admin:cmd</option>
            <option>PowerShell</option>
            <option>WSL:bash</option>
          </select>
        </DialogField>
        <SessionCommonOverviewFields draft={draft} onDraftChange={onDraftChange} prefs={prefs} updatePref={updatePref} />
        <DialogToggleField label="Sysmon:" checked={prefs.shellSysmon} onChange={(value) => updatePref("shellSysmon", value)} />
      </>
    );
  }

  if (activeProtocol === "SSH" || activeProtocol === "Tmux") {
    const ssh = draft.connection.kind === "ssh" || draft.connection.kind === "tmux" ? draft.connection : createSshConnection();
    const kind = activeProtocol === "Tmux" ? "tmux" : "ssh";
    return (
      <>
        <DialogField label="主机:(H)">
          <input
            value={formatSshTarget(ssh)}
            placeholder="[用户@]主机地址"
            onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...parseSshTarget(event.target.value, ssh), kind } })}
          />
        </DialogField>
        <DialogField label="端口:(P)">
          <input type="number" value={ssh.endpoint.port} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, endpoint: { ...ssh.endpoint, port: Number(event.target.value) } } })} />
        </DialogField>
        <SessionCommonOverviewFields draft={draft} onDraftChange={onDraftChange} prefs={prefs} updatePref={updatePref} />
        <DialogToggleField label="Sftp:" checked={draft.transfer.sftp} onChange={(value) => onDraftChange({ ...draft, transfer: { ...draft.transfer, sftp: value } })} />
        <DialogToggleField label="Sysmon:" checked={prefs.sshSysmon} onChange={(value) => updatePref("sshSysmon", value)} />
      </>
    );
  }

  if (activeProtocol === "Telnet") {
    const telnet = draft.connection.kind === "telnet" ? draft.connection : createTcpConnection("telnet");
    return (
      <>
        <DialogField label="主机:(H)">
          <input value={telnet.host} onChange={(event) => onDraftChange({ ...draft, kind: "telnet", connection: { ...telnet, kind: "telnet", host: event.target.value } })} />
        </DialogField>
        <DialogField label="端口:(P)">
          <input type="number" value={telnet.port} onChange={(event) => onDraftChange({ ...draft, kind: "telnet", connection: { ...telnet, kind: "telnet", port: Number(event.target.value) } })} />
        </DialogField>
        <SessionCommonOverviewFields draft={draft} onDraftChange={onDraftChange} prefs={prefs} updatePref={updatePref} />
      </>
    );
  }

  if (activeProtocol === "Tcp") {
    const tcp = draft.connection.kind === "tcp" ? draft.connection : createTcpConnection("tcp");
    return (
      <>
        <DialogField label="主机:(H)">
          <input value={tcp.host} onChange={(event) => onDraftChange({ ...draft, kind: "tcp", connection: { ...tcp, kind: "tcp", host: event.target.value } })} />
        </DialogField>
        <DialogField label="端口:(P)">
          <input type="number" value={tcp.port} onChange={(event) => onDraftChange({ ...draft, kind: "tcp", connection: { ...tcp, kind: "tcp", port: Number(event.target.value) } })} />
        </DialogField>
        <SessionCommonOverviewFields draft={draft} onDraftChange={onDraftChange} prefs={prefs} updatePref={updatePref} showData />
      </>
    );
  }

  const serial = draft.connection.kind === "serial" ? draft.connection : createSerialConnection();
  return (
    <>
      <DialogField label="串口:(S)">
        <select value={serial.port} onChange={(event) => onDraftChange({ ...draft, kind: "serial", connection: { ...serial, port: event.target.value } })}>
          {serialPortOptions(serial.port, serialPorts).map((option) => (
            <option key={option || "blank"} value={option}>
              {option || "选择串口"}
            </option>
          ))}
        </select>
      </DialogField>
      <SessionCommonOverviewFields draft={draft} onDraftChange={onDraftChange} prefs={prefs} updatePref={updatePref} showData />
    </>
  );
}

function SessionCommonOverviewFields({
  draft,
  onDraftChange,
  prefs,
  updatePref,
  showData = false,
}: {
  draft: SessionProfile;
  onDraftChange: (draft: SessionProfile) => void;
  prefs: SessionPrefs;
  updatePref: <K extends keyof SessionPrefs>(key: K, value: SessionPrefs[K]) => void;
  showData?: boolean;
}) {
  return (
    <>
      <DialogField label="OneKey:">
        <select value={prefs.oneKey} onChange={(event) => updatePref("oneKey", event.target.value)}>
          <option>无</option>
          <option>密码</option>
          <option>公钥</option>
          <option>Keyboard Interactive</option>
        </select>
      </DialogField>
      <DialogField label="标签:(L)">
        <input value={draft.tags.join(", ")} onChange={(event) => onDraftChange({ ...draft, tags: event.target.value.split(",").map((item) => item.trim()).filter(Boolean) })} />
      </DialogField>
      <DialogField label="分组:(G)">
        <input value={draft.group} onChange={(event) => onDraftChange({ ...draft, group: event.target.value })} placeholder="[嵌套组] a>b>c" />
      </DialogField>
      {showData ? (
        <DialogField label="数据:(D)">
          <select value={prefs.dataMode} onChange={(event) => updatePref("dataMode", event.target.value)}>
            <option>text</option>
            <option>binary</option>
          </select>
        </DialogField>
      ) : null}
      <DialogField label="终端:(T)">
        <select value={draft.terminal.term} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, term: event.target.value } })}>
          <option>xterm-256color</option>
          <option>xterm</option>
          <option>vt220</option>
        </select>
      </DialogField>
      <DialogField label="系统:(S)">
        <select value={prefs.system} onChange={(event) => updatePref("system", event.target.value)}>
          <option>cmd</option>
          <option>linux</option>
          <option>powershell</option>
          <option>bash</option>
        </select>
      </DialogField>
      <DialogField label="字符集:(C)">
        <select value={prefs.charset} onChange={(event) => updatePref("charset", event.target.value)}>
          <option>ISO-8859-1</option>
          <option>UTF-8</option>
          <option>GBK</option>
        </select>
      </DialogField>
      <DialogField label="描述:(D)">
        <textarea value={draft.name} onChange={(event) => onDraftChange({ ...draft, name: event.target.value })} />
      </DialogField>
    </>
  );
}

function ShellProcessFields({
  draft,
  onDraftChange,
  prefs,
  updatePref,
}: {
  draft: SessionProfile;
  onDraftChange: (draft: SessionProfile) => void;
  prefs: SessionPrefs;
  updatePref: <K extends keyof SessionPrefs>(key: K, value: SessionPrefs[K]) => void;
}) {
  const shell = draft.connection.kind === "shell" ? draft.connection : createShellConnection();
  return (
    <>
      <DialogField label="程序:(P)">
        <input value={shell.program} onChange={(event) => onDraftChange({ ...draft, kind: "shell", connection: { ...shell, program: event.target.value } })} />
      </DialogField>
      <DialogField label="参数:(A)">
        <input value={shell.args.join(" ")} onChange={(event) => onDraftChange({ ...draft, kind: "shell", connection: { ...shell, args: event.target.value.split(" ").filter(Boolean) } })} />
      </DialogField>
      <DialogField label="目录:(W)">
        <input value={shell.cwd ?? ""} onChange={(event) => onDraftChange({ ...draft, kind: "shell", connection: { ...shell, cwd: event.target.value || null } })} />
      </DialogField>
      <DialogField label="模式:(M)">
        <select value={prefs.shellLogin ? "login" : "plain"} onChange={(event) => updatePref("shellLogin", event.target.value === "login")}>
          <option value="plain">普通进程</option>
          <option value="login">Login shell</option>
        </select>
      </DialogField>
      <DialogField label="权限:(R)">
        <select value={prefs.shellElevated ? "admin" : "user"} onChange={(event) => updatePref("shellElevated", event.target.value === "admin")}>
          <option value="user">当前用户</option>
          <option value="admin">管理员</option>
        </select>
      </DialogField>
    </>
  );
}

function TriggerFields({ draft, onDraftChange }: { draft: SessionProfile; onDraftChange: (draft: SessionProfile) => void }) {
  const trigger = draft.triggers[0] ?? createDefaultTrigger();
  const matcher = trigger.matcher as { type?: string; text?: string; pattern?: string; case_sensitive?: boolean };
  const firstAction = trigger.actions[0] as { type?: string; label?: string; message?: string; color?: string; command?: string };

  function updateTrigger(patch: Partial<TriggerSpec>) {
    const next = { ...trigger, ...patch };
    onDraftChange({ ...draft, triggers: [next, ...draft.triggers.slice(1)] });
  }

  return (
    <>
      <DialogField label="启用:(E)">
        <select value={trigger.enabled ? "on" : "off"} onChange={(event) => updateTrigger({ enabled: event.target.value === "on" })}>
          <option value="on">开启</option>
          <option value="off">关闭</option>
        </select>
      </DialogField>
      <DialogField label="名称:(N)">
        <input value={trigger.label} onChange={(event) => updateTrigger({ label: event.target.value })} />
      </DialogField>
      <DialogField label="匹配:(M)">
        <select
          value={matcher.type === "regex" ? "regex" : "contains"}
          onChange={(event) => updateTrigger({
            matcher: event.target.value === "regex"
              ? { type: "regex", pattern: matcher.pattern || matcher.text || "" }
              : { type: "contains", text: matcher.text || matcher.pattern || "", case_sensitive: false },
          })}
        >
          <option value="contains">包含文本</option>
          <option value="regex">正则表达式</option>
        </select>
      </DialogField>
      <DialogField label="内容:(T)">
        <input
          value={matcher.type === "regex" ? matcher.pattern ?? "" : matcher.text ?? ""}
          onChange={(event) => updateTrigger({
            matcher: matcher.type === "regex"
              ? { type: "regex", pattern: event.target.value }
              : { type: "contains", text: event.target.value, case_sensitive: Boolean(matcher.case_sensitive) },
          })}
        />
      </DialogField>
      <DialogField label="大小写:(C)">
        <select
          value={matcher.case_sensitive ? "on" : "off"}
          onChange={(event) => updateTrigger({ matcher: { type: "contains", text: matcher.text ?? "", case_sensitive: event.target.value === "on" } })}
          disabled={matcher.type === "regex"}
        >
          <option value="off">忽略</option>
          <option value="on">区分</option>
        </select>
      </DialogField>
      <DialogField label="动作:(A)">
        <select
          value={firstAction.type ?? "timeline-mark"}
          onChange={(event) => updateTrigger({ actions: [defaultTriggerAction(event.target.value)] })}
        >
          <option value="timeline-mark">时间线标记</option>
          <option value="notification">通知</option>
          <option value="highlight">高亮</option>
          <option value="local-command">本地命令</option>
        </select>
      </DialogField>
      <DialogField label="参数:(P)">
        <input
          value={firstAction.label ?? firstAction.message ?? firstAction.color ?? firstAction.command ?? ""}
          onChange={(event) => updateTrigger({ actions: [patchTriggerAction(firstAction.type ?? "timeline-mark", event.target.value)] })}
        />
      </DialogField>
    </>
  );
}

function SshAdvancedFields({
  section,
  draft,
  onDraftChange,
  prefs,
  updatePref,
}: {
  section: string;
  draft: SessionProfile;
  onDraftChange: (draft: SessionProfile) => void;
  prefs: SessionPrefs;
  updatePref: <K extends keyof SessionPrefs>(key: K, value: SessionPrefs[K]) => void;
}) {
  const ssh = draft.connection.kind === "ssh" || draft.connection.kind === "tmux" ? draft.connection : createSshConnection();
  const kind = draft.connection.kind === "tmux" ? "tmux" : "ssh";
  const [vaultPrivateKey, setVaultPrivateKey] = useState("");
  const [vaultStatus, setVaultStatus] = useState("");
  const [vaultBusy, setVaultBusy] = useState(false);
  const [secretStatus, setSecretStatus] = useState("");
  const [hostKeyScan, setHostKeyScan] = useState<HostKeyScanResult | null>(null);
  const [hostKeyStatus, setHostKeyStatus] = useState("");
  const [jumpSecretDrafts, setJumpSecretDrafts] = useState<Record<string, string>>({});
  const [jumpStatus, setJumpStatus] = useState("");

  if (section === "连接") {
    const updateJump = (index: number, patch: Partial<JumpHop>) => {
      const jumps = ssh.jumps.map((jump, jumpIndex) => (jumpIndex === index ? { ...jump, ...patch } : jump));
      onDraftChange({ ...draft, kind, connection: { ...ssh, kind, jumps } });
    };
    const addJump = () => {
      const next: JumpHop = { host: "", port: 22, username: ssh.username, passwordSecretRef: null, passphraseSecretRef: null, identityRef: null, hostKeyPolicy: null };
      onDraftChange({ ...draft, kind, connection: { ...ssh, kind, jumps: [...ssh.jumps, next] } });
    };
    const removeJump = (index: number) => {
      onDraftChange({ ...draft, kind, connection: { ...ssh, kind, jumps: ssh.jumps.filter((_, jumpIndex) => jumpIndex !== index) } });
    };
    const updateJumpPolicy = (index: number, patch: Partial<HostKeyPolicy>) => {
      const jump = ssh.jumps[index];
      if (!jump) return;
      updateJump(index, { hostKeyPolicy: { ...createJumpHostKeyPolicy(jump), ...(jump.hostKeyPolicy ?? {}), ...patch } });
    };
    const jumpSecretKey = (index: number, field: "passwordSecretRef" | "passphraseSecretRef") => `${index}:${field}`;
    const setJumpSecretDraft = (index: number, field: "passwordSecretRef" | "passphraseSecretRef", value: string) => {
      setJumpSecretDrafts((current) => ({ ...current, [jumpSecretKey(index, field)]: value }));
    };
    const saveJumpSecret = async (index: number, field: "passwordSecretRef" | "passphraseSecretRef") => {
      const jump = ssh.jumps[index];
      if (!jump) return;
      const secret = jumpSecretDrafts[jumpSecretKey(index, field)] ?? "";
      if (!secret.trim()) return;
      setJumpStatus("");
      try {
        const currentRef = field === "passwordSecretRef" ? jump.passwordSecretRef : jump.passphraseSecretRef;
        const response = await invokeBackend<{ secretRef: string }>("save_secret", {
          request: { secretRef: currentRef ?? null, secret },
        });
        const patch: Partial<JumpHop> = field === "passwordSecretRef" ? { passwordSecretRef: response.secretRef } : { passphraseSecretRef: response.secretRef };
        updateJump(index, patch);
        setJumpSecretDrafts((current) => ({ ...current, [jumpSecretKey(index, field)]: "" }));
        setJumpStatus("已保存跳板凭据");
      } catch (error) {
        setJumpStatus(formatError(error));
      }
    };
    const deleteJumpSecret = async (index: number, field: "passwordSecretRef" | "passphraseSecretRef") => {
      const jump = ssh.jumps[index];
      const secretRef = field === "passwordSecretRef" ? jump?.passwordSecretRef : jump?.passphraseSecretRef;
      if (!secretRef) return;
      setJumpStatus("");
      try {
        await invokeBackend("delete_secret", { secretRef });
        const patch: Partial<JumpHop> = field === "passwordSecretRef" ? { passwordSecretRef: null } : { passphraseSecretRef: null };
        updateJump(index, patch);
        setJumpStatus("已删除跳板凭据");
      } catch (error) {
        setJumpStatus(formatError(error));
      }
    };
    return (
      <>
        <DialogField label="别名:(A)">
          <input value={ssh.hostKeyPolicy.alias ?? ""} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, hostKeyPolicy: { ...ssh.hostKeyPolicy, alias: event.target.value || null } } })} />
        </DialogField>
        <DialogField label="Jump Host:">
          <div className="jump-list">
            {ssh.jumps.map((jump, index) => {
              const policy = jump.hostKeyPolicy ?? createJumpHostKeyPolicy(jump);
              return (
                <div className="jump-hop" key={index}>
                  <div className="jump-hop-row">
                    <span className="jump-hop-index">{index + 1}</span>
                    <input value={jump.host} onChange={(event) => updateJump(index, { host: event.target.value })} placeholder="host" />
                    <input type="number" value={jump.port} onChange={(event) => updateJump(index, { port: Number(event.target.value) || 22 })} aria-label={`Jump ${index + 1} port`} />
                    <input value={jump.username} onChange={(event) => updateJump(index, { username: event.target.value })} placeholder="user" />
                    <input value={jump.identityRef ?? ""} onChange={(event) => updateJump(index, { identityRef: event.target.value || null })} placeholder="identity id" />
                    <button type="button" className="icon-button" onClick={() => removeJump(index)} title="删除跳板">
                      <X size={14} />
                    </button>
                  </div>
                  <div className="jump-hop-extra">
                    <input type="password" value={jumpSecretDrafts[jumpSecretKey(index, "passwordSecretRef")] ?? ""} onChange={(event) => setJumpSecretDraft(index, "passwordSecretRef", event.target.value)} placeholder="password" />
                    <button type="button" className="icon-button" onClick={() => void saveJumpSecret(index, "passwordSecretRef")} title="保存跳板密码">
                      <Lock size={14} />
                    </button>
                    <input value={jump.passwordSecretRef ?? ""} onChange={(event) => updateJump(index, { passwordSecretRef: event.target.value || null })} placeholder="password secretRef" />
                    <input type="password" value={jumpSecretDrafts[jumpSecretKey(index, "passphraseSecretRef")] ?? ""} onChange={(event) => setJumpSecretDraft(index, "passphraseSecretRef", event.target.value)} placeholder="passphrase" />
                    <button type="button" className="icon-button" onClick={() => void saveJumpSecret(index, "passphraseSecretRef")} title="保存跳板口令">
                      <Lock size={14} />
                    </button>
                    <input value={jump.passphraseSecretRef ?? ""} onChange={(event) => updateJump(index, { passphraseSecretRef: event.target.value || null })} placeholder="passphrase secretRef" />
                    <button type="button" className="icon-button" onClick={() => void deleteJumpSecret(index, "passwordSecretRef")} disabled={!jump.passwordSecretRef} title="删除跳板密码">
                      <X size={14} />
                    </button>
                    <button type="button" className="icon-button" onClick={() => void deleteJumpSecret(index, "passphraseSecretRef")} disabled={!jump.passphraseSecretRef} title="删除跳板口令">
                      <X size={14} />
                    </button>
                  </div>
                  <div className="jump-hop-policy">
                    <select value={jump.hostKeyPolicy ? "custom" : "inherit"} onChange={(event) => updateJump(index, { hostKeyPolicy: event.target.value === "custom" ? createJumpHostKeyPolicy(jump) : null })}>
                      <option value="inherit">继承</option>
                      <option value="custom">自定义</option>
                    </select>
                    {jump.hostKeyPolicy ? (
                      <>
                        <select value={policy.mode} onChange={(event) => updateJumpPolicy(index, { mode: event.target.value as HostKeyPolicy["mode"] })}>
                          <option value="strict">strict</option>
                          <option value="trust-on-first-use">trust-on-first-use</option>
                          <option value="ask-every-time">ask-every-time</option>
                        </select>
                        <input value={policy.alias ?? ""} onChange={(event) => updateJumpPolicy(index, { alias: event.target.value || null })} placeholder="host-key alias" />
                        <select value={policy.trustScope} onChange={(event) => updateJumpPolicy(index, { trustScope: event.target.value as HostKeyPolicy["trustScope"] })}>
                          <option value="profile">profile</option>
                          <option value="project">project</option>
                          <option value="user">user</option>
                        </select>
                        <label className="jump-hop-check">
                          <input type="checkbox" checked={policy.allowRotation} onChange={(event) => updateJumpPolicy(index, { allowRotation: event.target.checked })} />
                          <span>轮换</span>
                        </label>
                        <label className="jump-hop-check">
                          <input type="checkbox" checked={policy.checkIp} onChange={(event) => updateJumpPolicy(index, { checkIp: event.target.checked })} />
                          <span>IP</span>
                        </label>
                      </>
                    ) : null}
                  </div>
                </div>
              );
            })}
            {jumpStatus ? <span className="settings-inline-status">{jumpStatus}</span> : null}
            <button type="button" className="settings-secondary-button jump-add-button" onClick={addJump}>
              <Plus size={14} />
              <span>添加跳板</span>
            </button>
          </div>
        </DialogField>
        <DialogField label="KeepAlive:">
          <input value={prefs.sshKeepaliveSeconds} onChange={(event) => updatePref("sshKeepaliveSeconds", event.target.value)} />
        </DialogField>
        <DialogField label="断线重连:">
          <select value={ssh.reconnect ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, reconnect: event.target.value === "on" } })}>
            <option value="on">开启</option>
            <option value="off">关闭</option>
          </select>
        </DialogField>
        <DialogField label="压缩:(C)">
          <select value={prefs.sshCompression ? "on" : "off"} onChange={(event) => updatePref("sshCompression", event.target.value === "on")}>
            <option value="off">关闭</option>
            <option value="on">开启</option>
          </select>
        </DialogField>
      </>
    );
  }

  if (section === "代理") {
    return (
      <>
        <DialogField label="启用:(E)">
          <select value={prefs.sshProxyEnabled ? "on" : "off"} onChange={(event) => updatePref("sshProxyEnabled", event.target.value === "on")}>
            <option value="off">关闭</option>
            <option value="on">开启</option>
          </select>
        </DialogField>
        <DialogField label="类型:(T)">
          <select value={prefs.sshProxyType} onChange={(event) => updatePref("sshProxyType", event.target.value)}>
            <option>SOCKS5</option>
            <option>HTTP</option>
          </select>
        </DialogField>
        <DialogField label="主机:(H)">
          <input value={prefs.sshProxyHost} onChange={(event) => updatePref("sshProxyHost", event.target.value)} />
        </DialogField>
        <DialogField label="端口:(P)">
          <input value={prefs.sshProxyPort} onChange={(event) => updatePref("sshProxyPort", event.target.value)} />
        </DialogField>
      </>
    );
  }

  if (section === "验证") {
    const scanHostKey = async () => {
      setHostKeyStatus("");
      setHostKeyScan(null);
      try {
        const result = await invokeBackend<HostKeyScanResult>("scan_ssh_host_key", { profile: prepareSessionProfile(draft), password: null, passphrase: null });
        setHostKeyScan(result);
      } catch (error) {
        setHostKeyStatus(formatError(error));
      }
    };
    const trustHostKey = async (decision: HostKeyDecisionValue) => {
      if (!hostKeyScan) return;
      setHostKeyStatus("");
      try {
        const trusted = await invokeBackend<TrustedHostKey | null>("trust_scanned_host_key", {
          request: { profile: prepareSessionProfile(draft), observation: hostKeyScan.observation, decision },
        });
        if (trusted) {
          onDraftChange({ ...draft, kind, connection: { ...ssh, kind, trustedHostKeys: [trusted, ...ssh.trustedHostKeys.filter((key) => key.id !== trusted.id)] } });
        }
        setHostKeyStatus(decision === "trust-once" ? "已临时信任，下一次连接有效" : trusted ? `已信任 ${trusted.fingerprintSha256}` : "未写入配置");
      } catch (error) {
        setHostKeyStatus(formatError(error));
      }
    };
    return (
      <>
        <DialogField label="HostKey:">
          <select value={ssh.hostKeyPolicy.mode} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, hostKeyPolicy: { ...ssh.hostKeyPolicy, mode: event.target.value as "strict" | "trust-on-first-use" | "ask-every-time" } } })}>
            <option value="strict">strict</option>
            <option value="trust-on-first-use">trust-on-first-use</option>
            <option value="ask-every-time">ask-every-time</option>
          </select>
        </DialogField>
        <DialogField label="轮换:(R)">
          <select value={ssh.hostKeyPolicy.allowRotation ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, hostKeyPolicy: { ...ssh.hostKeyPolicy, allowRotation: event.target.value === "on" } } })}>
            <option value="off">阻断变更</option>
            <option value="on">允许追加</option>
          </select>
        </DialogField>
        <DialogField label="校验IP:(I)">
          <select value={ssh.hostKeyPolicy.checkIp ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, hostKeyPolicy: { ...ssh.hostKeyPolicy, checkIp: event.target.value === "on" } } })}>
            <option value="off">关闭</option>
            <option value="on">开启</option>
          </select>
        </DialogField>
        <DialogField label="信任域:(S)">
          <select value={ssh.hostKeyPolicy.trustScope} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, hostKeyPolicy: { ...ssh.hostKeyPolicy, trustScope: event.target.value as "profile" | "project" | "user" } } })}>
            <option value="profile">profile</option>
            <option value="project">project</option>
            <option value="user">user</option>
          </select>
        </DialogField>
        <DialogField label="扫描:">
          <div className="inline-actions">
            <button type="button" onClick={() => void scanHostKey()}>扫描 Host Key</button>
            <span>{hostKeyScan ? describeHostKeyEvaluation(hostKeyScan) : hostKeyStatus}</span>
          </div>
        </DialogField>
        {hostKeyScan ? (
          <DialogField label="处理:">
            <div className="inline-actions">
              <button type="button" onClick={() => void trustHostKey("trust-once")}>仅本次</button>
              <button type="button" onClick={() => void trustHostKey("append-to-profile")}>加入 Profile</button>
              <button type="button" onClick={() => void trustHostKey("append-to-project")}>加入 Project</button>
              <button type="button" onClick={() => void trustHostKey("replace-for-profile")}>替换 Profile</button>
            </div>
          </DialogField>
        ) : null}
      </>
    );
  }

  if (section === "代理人") {
    return (
      <>
        <DialogField label="Agent:(A)">
          <select value={ssh.agentPolicy.enabled ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, agentPolicy: { ...ssh.agentPolicy, enabled: event.target.value === "on" } } })}>
            <option value="off">禁用</option>
            <option value="on">启用</option>
          </select>
        </DialogField>
        <DialogField label="Forward:(F)">
          <select value={ssh.agentPolicy.forwarding ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, agentPolicy: { ...ssh.agentPolicy, forwarding: event.target.value === "on" } } })}>
            <option value="off">禁用</option>
            <option value="on">启用</option>
          </select>
        </DialogField>
        <DialogField label="Offer:(O)">
          <select value={ssh.agentPolicy.offerMode} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, agentPolicy: { ...ssh.agentPolicy, offerMode: event.target.value as "disabled" | "after-profile-keys" | "before-profile-keys" } } })}>
            <option value="disabled">disabled</option>
            <option value="after-profile-keys">after-profile-keys</option>
            <option value="before-profile-keys">before-profile-keys</option>
          </select>
        </DialogField>
      </>
    );
  }

  if (section === "密码") {
    const deleteSavedSecret = async (field: "passwordSecretRef" | "passphraseSecretRef") => {
      const secretRef = ssh[field];
      if (!secretRef) return;
      setSecretStatus("");
      try {
        await invokeBackend("delete_secret", { secretRef });
        onDraftChange({ ...draft, kind, connection: { ...ssh, kind, [field]: null } });
        setSecretStatus("已删除保存的凭据");
      } catch (error) {
        setSecretStatus(formatError(error));
      }
    };
    return (
      <>
        <DialogField label="密码:(P)">
          <select value={prefs.sshPasswordMode} onChange={(event) => updatePref("sshPasswordMode", event.target.value)}>
            <option>prompt</option>
            <option>vault</option>
            <option>disabled</option>
          </select>
        </DialogField>
        <DialogField label="交互:(K)">
          <select value={prefs.sshKeyboardInteractive ? "on" : "off"} onChange={(event) => updatePref("sshKeyboardInteractive", event.target.value === "on")}>
            <option value="on">允许</option>
            <option value="off">禁用</option>
          </select>
        </DialogField>
        <DialogField label="密码引用:">
          <div className="inline-actions">
            <input value={ssh.passwordSecretRef ?? ""} readOnly placeholder="未保存" />
            <button type="button" onClick={() => void deleteSavedSecret("passwordSecretRef")} disabled={!ssh.passwordSecretRef}>删除</button>
          </div>
        </DialogField>
        <DialogField label="口令引用:">
          <div className="inline-actions">
            <input value={ssh.passphraseSecretRef ?? ""} readOnly placeholder="未保存" />
            <button type="button" onClick={() => void deleteSavedSecret("passphraseSecretRef")} disabled={!ssh.passphraseSecretRef}>删除</button>
          </div>
        </DialogField>
        <DialogField label="状态:">
          <input value={secretStatus} readOnly placeholder="连接弹窗勾选保存后会生成引用" />
        </DialogField>
      </>
    );
  }

  if (section === "密钥交换") {
    return (
      <>
        <DialogField label="Kex:(K)">
          <select value={prefs.sshKexAlgorithm} onChange={(event) => updatePref("sshKexAlgorithm", event.target.value)}>
            <option>curve25519-sha256</option>
            <option>ecdh-sha2-nistp256</option>
            <option>diffie-hellman-group14-sha256</option>
          </select>
        </DialogField>
        <DialogField label="Cipher:(C)">
          <select value={prefs.sshCipherSuite} onChange={(event) => updatePref("sshCipherSuite", event.target.value)}>
            <option>chacha20-poly1305</option>
            <option>aes256-gcm</option>
            <option>aes128-ctr</option>
          </select>
        </DialogField>
        <DialogField label="Rekey:(R)">
          <input value={prefs.sshRekeyAfterMb} onChange={(event) => updatePref("sshRekeyAfterMb", event.target.value)} />
        </DialogField>
      </>
    );
  }

  if (section === "MAC 哈希") {
    return (
      <>
        <DialogField label="MAC:(M)">
          <select value={prefs.sshMacAlgorithm} onChange={(event) => updatePref("sshMacAlgorithm", event.target.value)}>
            <option>hmac-sha2-256-etm@openssh.com</option>
            <option>hmac-sha2-512-etm@openssh.com</option>
            <option>hmac-sha2-256</option>
          </select>
        </DialogField>
        <DialogField label="ETM:(E)">
          <select value={prefs.sshMacEtm ? "on" : "off"} onChange={(event) => updatePref("sshMacEtm", event.target.value === "on")}>
            <option value="on">优先</option>
            <option value="off">关闭</option>
          </select>
        </DialogField>
      </>
    );
  }

  if (section === "公钥") {
    const firstIdentity = ssh.identityRefs[0] ?? createIdentityRef();
    const updateIdentity = (patch: Partial<IdentityRef>) => {
      const identity = { ...firstIdentity, ...patch };
      onDraftChange({ ...draft, kind, connection: { ...ssh, kind, identityRefs: [identity, ...ssh.identityRefs.slice(1)] } });
    };
    const saveVaultPrivateKey = async () => {
      if (!vaultPrivateKey.trim()) return;
      setVaultBusy(true);
      setVaultStatus("");
      try {
        const response = await invokeBackend<{ secretRef: string }>("save_secret", {
          request: { secretRef: firstIdentity.secretRef ?? null, secret: vaultPrivateKey },
        });
        updateIdentity({ source: "profile-vault", secretRef: response.secretRef, path: null });
        setVaultPrivateKey("");
        setVaultStatus("已保存到系统密钥库");
      } catch (error) {
        setVaultStatus(formatError(error));
      } finally {
        setVaultBusy(false);
      }
    };
    const deleteVaultPrivateKey = async () => {
      if (!firstIdentity.secretRef) return;
      setVaultBusy(true);
      setVaultStatus("");
      try {
        await invokeBackend("delete_secret", { secretRef: firstIdentity.secretRef });
        updateIdentity({ secretRef: null });
        setVaultStatus("已从系统密钥库删除");
      } catch (error) {
        setVaultStatus(formatError(error));
      } finally {
        setVaultBusy(false);
      }
    };
    return (
      <>
        <DialogField label="身份:(I)">
          <select value={ssh.identityPolicy.identitiesOnly ? "only" : "agent"} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, identityPolicy: { ...ssh.identityPolicy, identitiesOnly: event.target.value === "only" } } })}>
            <option value="only">IdentitiesOnly</option>
            <option value="agent">Profile + Agent</option>
          </select>
        </DialogField>
        <DialogField label="顺序:(O)">
          <select value={ssh.identityPolicy.authOrder.join(">")} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, identityPolicy: { ...ssh.identityPolicy, authOrder: event.target.value.split(">") as AuthMethod[] } } })}>
            <option>public-key&gt;keyboard-interactive&gt;password</option>
            <option>public-key&gt;password</option>
            <option>password</option>
          </select>
        </DialogField>
        <DialogField label="公钥:(K)">
          <select value={firstIdentity.source} onChange={(event) => updateIdentity({ source: event.target.value as IdentityRef["source"] })}>
            <option>profile-vault</option>
            <option>system-file</option>
            <option>agent</option>
            <option>public-key-only</option>
          </select>
        </DialogField>
        <DialogField label="名称:(N)">
          <input value={firstIdentity.label} onChange={(event) => updateIdentity({ label: event.target.value })} />
        </DialogField>
        <DialogField label="私钥文件:(F)">
          <input value={firstIdentity.path ?? ""} onChange={(event) => updateIdentity({ path: event.target.value || null, source: event.target.value ? "system-file" : firstIdentity.source })} placeholder="~/.ssh/id_ed25519" />
        </DialogField>
        <DialogField label="Vault Ref:">
          <input value={firstIdentity.secretRef ?? ""} readOnly placeholder="保存 profile-vault 私钥后生成" />
        </DialogField>
        {firstIdentity.source === "profile-vault" ? (
          <DialogField label="私钥内容:">
            <textarea value={vaultPrivateKey} onChange={(event) => setVaultPrivateKey(event.target.value)} placeholder="粘贴 OpenSSH 私钥，保存后只保留 secretRef" />
          </DialogField>
        ) : null}
        {firstIdentity.source === "profile-vault" ? (
          <DialogField label="密钥库:">
            <div className="inline-actions">
              <button type="button" onClick={() => void saveVaultPrivateKey()} disabled={vaultBusy || !vaultPrivateKey.trim()}>保存到系统密钥库</button>
              <button type="button" onClick={() => void deleteVaultPrivateKey()} disabled={vaultBusy || !firstIdentity.secretRef}>删除</button>
              <span>{vaultStatus}</span>
            </div>
          </DialogField>
        ) : null}
        <DialogField label="指纹:(P)">
          <input value={firstIdentity.fingerprintSha256 ?? ""} onChange={(event) => updateIdentity({ fingerprintSha256: event.target.value || null })} placeholder="SHA256:..." />
        </DialogField>
      </>
    );
  }

  if (section === "SFTP") {
    return (
      <>
        <DialogToggleField label="Sftp:" checked={draft.transfer.sftp} onChange={(value) => onDraftChange({ ...draft, transfer: { ...draft.transfer, sftp: value } })} />
        <DialogToggleField label="Scp:" checked={draft.transfer.scp} onChange={(value) => onDraftChange({ ...draft, transfer: { ...draft.transfer, scp: value } })} />
        <DialogField label="限速 B/s:">
          <input type="number" min={0} value={draft.transfer.rateLimitBytesPerSecond ?? 0} onChange={(event) => onDraftChange({ ...draft, transfer: { ...draft.transfer, rateLimitBytesPerSecond: Number(event.target.value) > 0 ? Number(event.target.value) : null } })} />
        </DialogField>
        <DialogField label="目录:(D)">
          <input value={draft.transfer.defaultLocalDir ?? ""} onChange={(event) => onDraftChange({ ...draft, transfer: { ...draft.transfer, defaultLocalDir: event.target.value || null } })} />
        </DialogField>
      </>
    );
  }

  return (
    <>
      <DialogToggleField label="X11:" checked={prefs.sshX11Enabled} onChange={(value) => updatePref("sshX11Enabled", value)} />
      <DialogField label="Display:">
        <input value={prefs.sshX11Display} onChange={(event) => updatePref("sshX11Display", event.target.value)} />
      </DialogField>
      <DialogField label="可信:(T)">
        <select value={prefs.sshX11Trusted ? "on" : "off"} onChange={(event) => updatePref("sshX11Trusted", event.target.value === "on")}>
          <option value="on">Trusted</option>
          <option value="off">Untrusted</option>
        </select>
      </DialogField>
    </>
  );
}

function TcpLikeAdvancedFields({
  protocol,
  section,
  draft,
  onDraftChange,
  prefs,
  updatePref,
}: {
  protocol: "Telnet" | "Tcp";
  section: string;
  draft: SessionProfile;
  onDraftChange: (draft: SessionProfile) => void;
  prefs: SessionPrefs;
  updatePref: <K extends keyof SessionPrefs>(key: K, value: SessionPrefs[K]) => void;
}) {
  const kind = protocol === "Telnet" ? "telnet" : "tcp";
  const tcp = draft.connection.kind === kind ? draft.connection : createTcpConnection(kind);

  if (section === "连接") {
    return (
      <>
        <DialogField label="重连:(R)">
          <select value={tcp.reconnect ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...tcp, kind, reconnect: event.target.value === "on" } })}>
            <option value="on">开启</option>
            <option value="off">关闭</option>
          </select>
        </DialogField>
        <DialogField label="延迟:(D)">
          <input value={String(prefs.reconnectDelayMs)} onChange={(event) => updatePref("reconnectDelayMs", Number(event.target.value))} />
        </DialogField>
        {protocol === "Telnet" ? (
          <DialogField label="回显:(E)">
            <select value={prefs.telnetLocalEcho ? "on" : "off"} onChange={(event) => updatePref("telnetLocalEcho", event.target.value === "on")}>
              <option value="off">远端控制</option>
              <option value="on">本地回显</option>
            </select>
          </DialogField>
        ) : (
          <DialogField label="KeepAlive:">
            <select value={prefs.tcpKeepalive ? "on" : "off"} onChange={(event) => updatePref("tcpKeepalive", event.target.value === "on")}>
              <option value="off">关闭</option>
              <option value="on">开启</option>
            </select>
          </DialogField>
        )}
      </>
    );
  }

  return (
    <>
      <DialogField label="启用:(E)">
        <select value={protocol === "Telnet" ? (prefs.telnetProxyEnabled ? "on" : "off") : prefs.tcpProxyEnabled ? "on" : "off"} onChange={(event) => protocol === "Telnet" ? updatePref("telnetProxyEnabled", event.target.value === "on") : updatePref("tcpProxyEnabled", event.target.value === "on")}>
          <option value="off">关闭</option>
          <option value="on">开启</option>
        </select>
      </DialogField>
      <DialogField label="类型:(T)">
        <select value={protocol === "Telnet" ? prefs.telnetProxyType : prefs.tcpProxyType} onChange={(event) => protocol === "Telnet" ? updatePref("telnetProxyType", event.target.value) : updatePref("tcpProxyType", event.target.value)}>
          <option>SOCKS5</option>
          <option>HTTP</option>
        </select>
      </DialogField>
      <DialogField label="主机:(H)">
        <input value={protocol === "Telnet" ? prefs.telnetProxyHost : prefs.tcpProxyHost} onChange={(event) => protocol === "Telnet" ? updatePref("telnetProxyHost", event.target.value) : updatePref("tcpProxyHost", event.target.value)} />
      </DialogField>
      <DialogField label="端口:(P)">
        <input value={protocol === "Telnet" ? prefs.telnetProxyPort : prefs.tcpProxyPort} onChange={(event) => protocol === "Telnet" ? updatePref("telnetProxyPort", event.target.value) : updatePref("tcpProxyPort", event.target.value)} />
      </DialogField>
    </>
  );
}

function SerialAdvancedFields({
  draft,
  onDraftChange,
  prefs,
  updatePref,
}: {
  draft: SessionProfile;
  onDraftChange: (draft: SessionProfile) => void;
  prefs: SessionPrefs;
  updatePref: <K extends keyof SessionPrefs>(key: K, value: SessionPrefs[K]) => void;
}) {
  const serial = draft.connection.kind === "serial" ? draft.connection : createSerialConnection();
  const update = (patch: Partial<ReturnType<typeof createSerialConnection>>) => onDraftChange({ ...draft, kind: "serial", connection: { ...serial, ...patch } });

  return (
    <>
      <DialogField label="波特率:(B)">
        <input type="number" value={serial.baudRate} onChange={(event) => update({ baudRate: Number(event.target.value) })} />
      </DialogField>
      <DialogField label="数据位:(D)">
        <select value={serial.dataBits} onChange={(event) => update({ dataBits: Number(event.target.value) })}>
          <option value={5}>5</option>
          <option value={6}>6</option>
          <option value={7}>7</option>
          <option value={8}>8</option>
        </select>
      </DialogField>
      <DialogField label="停止位:(S)">
        <select value={serial.stopBits} onChange={(event) => update({ stopBits: Number(event.target.value) })}>
          <option value={1}>1</option>
          <option value={2}>2</option>
        </select>
      </DialogField>
      <DialogField label="校验:(P)">
        <select value={serial.parity} onChange={(event) => update({ parity: event.target.value })}>
          <option>none</option>
          <option>odd</option>
          <option>even</option>
        </select>
      </DialogField>
      <DialogField label="流控:(F)">
        <select value={serial.flowControl} onChange={(event) => update({ flowControl: event.target.value })}>
          <option>none</option>
          <option>software</option>
          <option>hardware</option>
        </select>
      </DialogField>
      <DialogField label="DTR:(D)">
        <select value={serial.dtr ? "on" : "off"} onChange={(event) => update({ dtr: event.target.value === "on" })}>
          <option value="off">关闭</option>
          <option value="on">开启</option>
        </select>
      </DialogField>
      <DialogField label="RTS:(R)">
        <select value={serial.rts ? "on" : "off"} onChange={(event) => update({ rts: event.target.value === "on" })}>
          <option value="off">关闭</option>
          <option value="on">开启</option>
        </select>
      </DialogField>
      <DialogField label="重连:(R)">
        <select value={serial.reconnect ? "on" : "off"} onChange={(event) => update({ reconnect: event.target.value === "on" })}>
          <option value="on">开启</option>
          <option value="off">关闭</option>
        </select>
      </DialogField>
      <DialogField label="换行:(N)">
        <select value={prefs.serialNewline} onChange={(event) => updatePref("serialNewline", event.target.value)}>
          <option>CRLF</option>
          <option>LF</option>
          <option>CR</option>
        </select>
      </DialogField>
    </>
  );
}

function DialogFrame({ title, className, onClose, children }: { title: string; className: string; onClose: () => void; children: React.ReactNode }) {
  return (
    <div className="dialog-backdrop">
      <section className={`wind-dialog ${className}`}>
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>{title}</strong>
          <button onClick={onClose}><X size={22} /></button>
        </header>
        {children}
      </section>
    </div>
  );
}

function DialogField({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="dialog-field">
      <span>{label}</span>
      {children}
    </label>
  );
}

function DialogToggleField({ label, checked, onChange }: { label: string; checked: boolean; onChange: (value: boolean) => void }) {
  return (
    <label className="dialog-field dialog-toggle-field">
      <span>{label}</span>
      <button type="button" className={checked ? "switch-toggle on" : "switch-toggle"} onClick={() => onChange(!checked)} aria-pressed={checked}>
        <span />
      </button>
    </label>
  );
}

function SettingsSection({ title, children }: { title?: string; children: React.ReactNode }) {
  return (
    <section className="settings-section">
      {title ? <h2>{title}</h2> : null}
      <div className="settings-box">{children}</div>
    </section>
  );
}

function SettingRadio({ label, checked, name, onChange }: { label: string; checked: boolean; name: string; onChange: () => void }) {
  return (
    <label className="setting-radio">
      <input type="radio" name={name} checked={checked} onChange={onChange} />
      <span>{label}</span>
    </label>
  );
}

function SettingCheck({ label, checked, onChange }: { label: string; checked: boolean; onChange: (value: boolean) => void }) {
  return (
    <label className="setting-check">
      <input type="checkbox" checked={checked} onChange={(event) => onChange(event.target.checked)} />
      <span>{label}</span>
    </label>
  );
}

function SettingInput({ label, value, type = "text", placeholder, onChange }: { label: string; value: string | number; type?: string; placeholder?: string; onChange: (value: string) => void }) {
  return (
    <label className="setting-row">
      <span>{label}</span>
      <input type={type} value={value} placeholder={placeholder} onChange={(event) => onChange(event.target.value)} />
    </label>
  );
}

function SettingSelect({ label, value, options, onChange }: { label: string; value: string; options: string[]; onChange: (value: string) => void }) {
  return (
    <label className="setting-row">
      <span>{label}</span>
      <select value={value} onChange={(event) => onChange(event.target.value)}>
        {options.map((option) => (
          <option key={option || "blank"} value={option}>
            {option || "未指定"}
          </option>
        ))}
      </select>
    </label>
  );
}

function SettingButtonRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="setting-row">
      <span>{label}</span>
      <div className="setting-row-actions">{children}</div>
    </div>
  );
}

function createTerminalPrefs() {
  return {
    language: "Chinese (Simplified) - 中文（简体）",
    windowOpacity: "1.00",
    startupMode: "last",
    startupSessions: ["", "", "", ""],
    closeTabConfirm: true,
    closeWindowConfirm: true,
    theme: "dige-black",
    accent: "teal",
    layoutDensity: "标准",
    compactChrome: true,
    showStatusBar: true,
    highlightActiveTab: true,
    proxyEnabled: false,
    proxyType: "SOCKS5",
    proxyHost: "127.0.0.1",
    proxyPort: "1080",
    proxyDns: true,
    lockOnIdle: false,
    confirmPaste: true,
    maskSecrets: true,
    requireMasterPassword: false,
    tabPosition: "顶部",
    tabCloseConfirm: true,
    tabShowColors: true,
    groupColorBar: true,
    defaultTerm: "xterm-256color",
    terminalScrollback: 200000,
    pasteBracketed: true,
    completionEnabled: true,
    oneKeyCompletion: true,
    completionCommandNames: true,
    completionCommandOptions: true,
    completionCommandArgs: true,
    completionHistory: true,
    completionQuickCommands: true,
    completionTriggerChars: "1 字符",
    completionListHeight: "7 行",
    completionPreviewMode: "无处",
    historyEnabled: true,
    historyRetentionDays: "30",
    historyLimit: "10000",
    mouseReporting: true,
    mouseCopyOnSelect: true,
    rightClickPaste: true,
    binaryView: "Hex",
    binaryRadix: "16",
    binaryColumns: "16",
    binaryDividerColumns: "4",
    binaryGroupChars: "1",
    binaryAlternateRows: true,
    cursorShape: "块",
    cursorBlink: false,
    cursorBlinkPeriod: "0.5 秒",
    cursorWidth: "1.5 像素",
    fontFamily1: "Roboto Mono",
    fontFamily2: "",
    fontFamily3: "",
    fontFamily4: "",
    fontWeight: "常规",
    fontSize: "10 像素",
    preferThemeFont: true,
    highlightCursorLine: true,
    highlightCursor: true,
    highlightFold: true,
    highlightPairs: true,
    highlightIncrementalSearch: true,
    wrapLines: false,
    wrapColumnMode: "窗口边界",
    fileManagerEnabled: true,
    quickBarEnabled: true,
    xServerEnabled: false,
    extensionsEnabled: false,
  };
}

function createSessionPrefs() {
  return {
    oneKey: "无",
    shellPreset: "admin:cmd",
    shellSysmon: true,
    shellLogin: false,
    shellElevated: false,
    dataMode: "text",
    system: "cmd",
    charset: "ISO-8859-1",
    bellMode: "visual",
    visualBell: true,
    localEcho: false,
    syncInput: false,
    focusMode: false,
    backspaceMode: "DEL",
    altSendsEscape: true,
    confirmPaste: true,
    splitMode: "none",
    tabColor: "#5eead4",
    copyOnSelect: true,
    rightClickPaste: false,
    reconnectPolicy: "on-disconnect",
    reconnectDelayMs: 1000,
    sshSysmon: false,
    sshKeepaliveSeconds: "30",
    sshCompression: false,
    sshProxyEnabled: false,
    sshProxyType: "SOCKS5",
    sshProxyHost: "127.0.0.1",
    sshProxyPort: "1080",
    sshPasswordMode: "prompt",
    sshKeyboardInteractive: true,
    sshKexAlgorithm: "curve25519-sha256",
    sshCipherSuite: "chacha20-poly1305",
    sshRekeyAfterMb: "1024 MB",
    sshMacAlgorithm: "hmac-sha2-256-etm@openssh.com",
    sshMacEtm: true,
    sshPublicKeySource: "profile-vault",
    sshX11Enabled: false,
    sshX11Display: "localhost:10.0",
    sshX11Trusted: false,
    telnetLocalEcho: false,
    telnetProxyEnabled: false,
    telnetProxyType: "SOCKS5",
    telnetProxyHost: "127.0.0.1",
    telnetProxyPort: "1080",
    tcpKeepalive: false,
    tcpProxyEnabled: false,
    tcpProxyType: "SOCKS5",
    tcpProxyHost: "127.0.0.1",
    tcpProxyPort: "1080",
    serialNewline: "CRLF",
  };
}

function createSessionDraft(): SessionProfile {
  return {
    id: createSessionId(),
    name: "",
    kind: "serial",
    group: "",
    tags: [],
    connection: createSerialConnection(),
    terminal: {
      term: "xterm-256color",
      rows: 32,
      cols: 120,
      scrollback: 200000,
      fontFamily: "JetBrains Mono, monospace",
      fontSize: 13,
      theme: "portmate-dark",
    },
    logging: {
      enabled: false,
      raw: false,
      text: true,
      jsonl: true,
      redactSecrets: true,
      pathTemplate: "{profile}/{date}/{session}.jsonl",
    },
    triggers: [],
    transfer: { sftp: true, scp: true, xmodem: true, ymodem: true, zmodem: true, rateLimitBytesPerSecond: null, defaultLocalDir: null },
  };
}

function prepareSessionProfile(profile: SessionProfile): SessionProfile {
  const id = profile.id && profile.id !== "draft" ? profile.id : createSessionId();
  const name = profile.name.trim() || defaultSessionName(profile);
  const connection = normalizeConnectionConfig(profile.connection, id);
  return {
    ...profile,
    id,
    name,
    kind: connection.kind,
    connection,
    group: profile.group.trim(),
    tags: profile.tags.map((tag) => tag.trim()).filter(Boolean),
    terminal: {
      ...profile.terminal,
      term: profile.terminal.term.trim() || "xterm-256color",
    },
  };
}

function normalizeConnectionConfig(connection: ConnectionConfig, profileId: string): ConnectionConfig {
  if (connection.kind !== "ssh" && connection.kind !== "tmux") {
    return connection;
  }
  const alias = connection.hostKeyPolicy.alias?.trim();
  return {
    ...connection,
    jumps: connection.jumps
      .map((jump) => ({
        host: jump.host.trim(),
        port: Number.isFinite(jump.port) && jump.port > 0 ? Math.min(65535, Math.trunc(jump.port)) : 22,
        username: jump.username.trim() || connection.username.trim(),
        passwordSecretRef: jump.passwordSecretRef?.trim() || null,
        passphraseSecretRef: jump.passphraseSecretRef?.trim() || null,
        identityRef: jump.identityRef?.trim() || null,
        hostKeyPolicy: normalizeOptionalHostKeyPolicy(jump.hostKeyPolicy),
      }))
      .filter((jump) => jump.host),
    hostKeyPolicy: {
      ...connection.hostKeyPolicy,
      alias: alias || profileId,
    },
    trustedHostKeys: connection.trustedHostKeys.filter((key) => key.scope !== "profile" || !key.profileId || key.profileId === profileId),
    identityPolicy: {
      ...connection.identityPolicy,
      authOrder: connection.identityPolicy.authOrder.map(normalizeAuthMethod).filter((method, index, methods) => methods.indexOf(method) === index),
      lastSuccessful: connection.identityPolicy.lastSuccessful ? normalizeAuthMethod(connection.identityPolicy.lastSuccessful) : null,
    },
  };
}

function normalizeOptionalHostKeyPolicy(policy?: HostKeyPolicy | null): HostKeyPolicy | null {
  if (!policy) return null;
  const alias = policy.alias?.trim();
  return {
    ...policy,
    alias: alias || null,
  };
}

function createJumpHostKeyPolicy(jump?: JumpHop): HostKeyPolicy {
  const host = jump?.host.trim();
  const port = jump?.port && Number.isFinite(jump.port) ? Math.trunc(jump.port) : 22;
  return {
    mode: "trust-on-first-use",
    alias: host ? `jump:${host}:${port}` : null,
    trustScope: "profile",
    allowRotation: false,
    checkIp: false,
  };
}

function isolateDuplicatedConnection(profileId: string, connection: ConnectionConfig): ConnectionConfig {
  if (connection.kind !== "ssh" && connection.kind !== "tmux") {
    return connection;
  }
  return {
    ...connection,
    hostKeyPolicy: {
      ...connection.hostKeyPolicy,
      alias: profileId,
      trustScope: "profile",
    },
    trustedHostKeys: [],
    identityPolicy: {
      ...connection.identityPolicy,
      lastSuccessful: null,
    },
  };
}

function normalizeAuthMethod(method: string): AuthMethod {
  if (method === "publickey") return "public-key";
  if (method === "gssapi") return "gssapi-with-mic";
  if (method === "keyboard-interactive" || method === "password" || method === "public-key" || method === "gssapi-with-mic" || method === "none") {
    return method;
  }
  return "password";
}

function defaultSessionName(profile: SessionProfile) {
  const connection = profile.connection;
  switch (connection.kind) {
    case "shell":
      return connection.program || "Shell";
    case "ssh":
    case "tmux":
      return formatSshTarget(connection) || "SSH";
    case "telnet":
      return connection.host ? `telnet://${connection.host}:${connection.port}` : "Telnet";
    case "tcp":
      return connection.host ? `tcp://${connection.host}:${connection.port}` : "Tcp";
    case "serial":
      return connection.port || "Serial";
  }
}

function createSessionId() {
  const random = typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : `${Date.now()}-${Math.random().toString(16).slice(2)}`;
  return `session-${random}`;
}

function createSessionSummary(profile: SessionProfile): SessionSummary {
  const now = new Date().toISOString();
  return {
    profile,
    runtime: {
      sessionId: profile.id,
      paneId: `${profile.id}:main`,
      status: "disconnected",
      title: profile.name,
      cwd: null,
      connectedSince: null,
      lastActivity: now,
      lastDisconnect: null,
      lastDisconnectReason: null,
      activeTransport: profile.kind,
    },
    logLines: 0,
    lastLine: null,
  };
}

function createDefaultTrigger(): TriggerSpec {
  return {
    id: `trigger-${typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : Date.now()}`,
    label: "关键输出",
    matcher: { type: "contains", text: "error", case_sensitive: false },
    actions: [{ type: "timeline-mark", label: "error" }],
    enabled: true,
  };
}

function defaultTriggerAction(type: string) {
  switch (type) {
    case "notification":
      return { type, message: "触发器命中" };
    case "highlight":
      return { type, color: "#f4b860" };
    case "local-command":
      return { type, command: "" };
    case "timeline-mark":
    default:
      return { type: "timeline-mark", label: "mark" };
  }
}

function patchTriggerAction(type: string, value: string) {
  switch (type) {
    case "notification":
      return { type, message: value };
    case "highlight":
      return { type, color: value };
    case "local-command":
      return { type, command: value };
    case "timeline-mark":
    default:
      return { type: "timeline-mark", label: value };
  }
}

function setSessionStatus(session: SessionSummary, status: SessionStatus): SessionSummary {
  const now = new Date().toISOString();
  return {
    ...session,
    runtime: {
      ...session.runtime,
      status,
      title: session.profile.name,
      connectedSince: status === "connected" ? session.runtime.connectedSince ?? now : null,
      lastActivity: now,
      lastDisconnect: status === "connected" ? session.runtime.lastDisconnect ?? null : now,
      lastDisconnectReason: status === "connected" ? session.runtime.lastDisconnectReason ?? null : `session ${status}`,
      activeTransport: session.profile.kind,
    },
  };
}

function createLocalSystemEvent(profile: SessionProfile, text: string): SessionEvent {
  const now = new Date().toISOString();
  return {
    id: `event-${now}-${Math.random().toString(16).slice(2)}`,
    sessionId: profile.id,
    paneId: `${profile.id}:main`,
    ts: now,
    direction: "system",
    stream: "control",
    bytesRef: null,
    text,
    annotations: {},
  };
}

function applyConnectionCredentials(profile: SessionProfile, credentials: ConnectionCredentials): SessionProfile {
  if (!credentials.username || !isSshLikeProfile(profile)) {
    return profile;
  }
  return {
    ...profile,
    connection: {
      ...profile.connection,
      username: credentials.username,
    },
  };
}

async function persistConnectionSecrets(profile: SessionProfile, credentials: ConnectionCredentials): Promise<SessionProfile> {
  if (!isBackendAvailable() || !isSshLikeProfile(profile)) {
    return profile;
  }
  let connection = profile.connection;
  if (credentials.savePassword && credentials.password) {
    const response = await invokeBackend<{ secretRef: string }>("save_secret", {
      request: { secretRef: connection.passwordSecretRef ?? null, secret: credentials.password },
    });
    connection = { ...connection, passwordSecretRef: response.secretRef };
  }
  if (credentials.savePassphrase && credentials.passphrase) {
    const response = await invokeBackend<{ secretRef: string }>("save_secret", {
      request: { secretRef: connection.passphraseSecretRef ?? null, secret: credentials.passphrase },
    });
    connection = { ...connection, passphraseSecretRef: response.secretRef };
  }
  return { ...profile, connection };
}

function isSshLikeProfile(profile: SessionProfile): profile is SessionProfile & { connection: Extract<ConnectionConfig, { kind: "ssh" | "tmux" }> } {
  return profile.connection.kind === "ssh" || profile.connection.kind === "tmux";
}

function identityStableKey(identity: IdentityRef) {
  if (identity.fingerprintSha256) return `fingerprint:${identity.fingerprintSha256}`;
  if (identity.secretRef) return `secret:${identity.secretRef}`;
  if (identity.path) return `path:${identity.source}:${identity.path}`;
  return `id:${identity.id}`;
}

function clientIdentitySelectionId(profileId: string, identity: IdentityRef, index: number) {
  return `${profileId}\0${identity.id}\0${index}`;
}

function identitySourceLabel(source: IdentityRef["source"]) {
  switch (source) {
    case "profile-vault":
      return "Profile Vault";
    case "system-file":
      return "System File";
    case "agent":
      return "SSH Agent";
    case "public-key-only":
      return "Public Key";
  }
}

function groupClientIdentityItems(items: ClientIdentityItem[], groupBy: ClientIdentityGroupBy) {
  const groups = new Map<string, { id: string; label: string; items: ClientIdentityItem[] }>();
  for (const item of items) {
    const id = groupBy === "profile" ? item.profileId : item.identity.source;
    const label = groupBy === "profile" ? item.profileName : identitySourceLabel(item.identity.source);
    const group = groups.get(id) ?? { id, label, items: [] };
    group.items.push(item);
    groups.set(id, group);
  }
  return Array.from(groups.values());
}

function createLocalId() {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) return crypto.randomUUID();
  return `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function isHostKeyFailure(message: string) {
  const lower = message.toLowerCase();
  return lower.includes("host key") || message.includes("指纹") || message.includes("未受信任") || message.includes("已变化");
}

function formatError(error: unknown) {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}

function logSignature(events: SessionEvent[]) {
  const last = events[events.length - 1];
  return `${events.length}:${last?.id ?? ""}:${last?.ts ?? ""}`;
}

function sessionsSignature(sessions: SessionSummary[]) {
  return sessions.map((session) => `${session.profile.id}:${session.runtime.status}:${session.runtime.lastActivity}:${session.logLines}`).join("|");
}

function mergeSessionSummaries(current: SessionSummary[], saved: SessionSummary) {
  const index = current.findIndex((session) => session.profile.id === saved.profile.id);
  if (index < 0) return [...current, saved];
  return current.map((session, itemIndex) => itemIndex === index ? saved : session);
}

function mergeTunnels(current: TunnelStatus[], saved: TunnelStatus) {
  const index = current.findIndex((tunnel) => tunnel.spec.id === saved.spec.id);
  if (index < 0) return [...current, saved];
  return current.map((tunnel, itemIndex) => itemIndex === index ? saved : tunnel);
}

function emptyTunnelStatus(spec: TunnelSpec): TunnelStatus {
  return {
    spec,
    activeConnections: 0,
    totalConnections: 0,
    tcpToSshBytes: 0,
    sshToTcpBytes: 0,
    lastActivity: null,
    lastError: null,
  };
}

function loadLocalSessionSummaries() {
  try {
    const raw = window.localStorage.getItem("portmate.sessions");
    return raw ? JSON.parse(raw) as SessionSummary[] : emptySessions;
  } catch {
    return emptySessions;
  }
}

function saveLocalSessionSummaries(sessions: SessionSummary[]) {
  window.localStorage.setItem("portmate.sessions", JSON.stringify(sessions));
}

function loadLocalValue<T>(key: string, fallback: T): T {
  try {
    const raw = window.localStorage.getItem(key);
    return raw ? JSON.parse(raw) as T : fallback;
  } catch {
    return fallback;
  }
}

function saveLocalValue<T>(key: string, value: T) {
  window.localStorage.setItem(key, JSON.stringify(value));
}

function cloneSessionProfile(profile: SessionProfile): SessionProfile {
  return JSON.parse(JSON.stringify(profile)) as SessionProfile;
}

function createSerialConnection(): Extract<ConnectionConfig, { kind: "serial" }> {
  return {
    kind: "serial",
    port: "",
    baudRate: 115200,
    dataBits: 8,
    stopBits: 1,
    parity: "none",
    flowControl: "none",
    dtr: false,
    rts: false,
    reconnect: true,
  };
}

function createShellConnection(): Extract<ConnectionConfig, { kind: "shell" }> {
  return {
    kind: "shell",
    program: "",
    args: [],
    cwd: null,
  };
}

function createTcpConnection(kind: "telnet" | "tcp"): Extract<ConnectionConfig, { kind: "telnet" | "tcp" }> {
  return {
    kind,
    host: "",
    port: kind === "telnet" ? 23 : 0,
    reconnect: true,
  };
}

function createSshConnection(): Extract<ConnectionConfig, { kind: "ssh" | "tmux" }> {
  return {
    kind: "ssh",
    endpoint: { host: "", port: 22 },
    username: "",
    reconnect: true,
    passwordSecretRef: null,
    passphraseSecretRef: null,
    hostKeyPolicy: {
      mode: "trust-on-first-use",
      alias: null,
      trustScope: "profile",
      allowRotation: false,
      checkIp: false,
    },
    trustedHostKeys: [],
    identityPolicy: {
      identitiesOnly: true,
      authOrder: ["public-key", "keyboard-interactive", "password"],
      recordSuccess: true,
      lastSuccessful: null,
    },
    identityRefs: [],
    agentPolicy: {
      enabled: false,
      forwarding: false,
      offerMode: "after-profile-keys",
    },
    jumps: [],
    tunnels: [],
  };
}

function createIdentityRef(): IdentityRef {
  return {
    id: `identity-${typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : Date.now()}`,
    label: "profile key",
    source: "system-file",
    fingerprintSha256: null,
    path: null,
    secretRef: null,
  };
}

function createMcpGrant(): McpGrant {
  return {
    clientId: "portmate-local",
    name: "Local MCP Client",
    scopes: ["read-sessions", "read-logs"],
    allowedSessions: [],
    expiresAt: null,
    revokedAt: null,
  };
}

function flattenSessionTree(tree: readonly SessionTreeNode[]) {
  return tree.flatMap((item) => (item.children ? [item.label, ...item.children] : [item.label]));
}

function serialPortOptions(current: string, discovered: string[]) {
  return Array.from(new Set([current, ...discovered, "COM1", "COM2", "COM3", "COM7", "/dev/ttyUSB0", "/dev/ttyACM0"].filter(Boolean)));
}

function formatSshTarget(ssh: Extract<ConnectionConfig, { kind: "ssh" | "tmux" }>) {
  return ssh.username ? `${ssh.username}@${ssh.endpoint.host}` : ssh.endpoint.host;
}

function parseSshTarget(value: string, ssh: Extract<ConnectionConfig, { kind: "ssh" | "tmux" }>): Extract<ConnectionConfig, { kind: "ssh" | "tmux" }> {
  const at = value.lastIndexOf("@");
  if (at > 0) {
    return { ...ssh, kind: "ssh", username: value.slice(0, at), endpoint: { ...ssh.endpoint, host: value.slice(at + 1) } };
  }
  return { ...ssh, kind: "ssh", endpoint: { ...ssh.endpoint, host: value } };
}

function protocolFromKind(kind: SessionKind): ProtocolTab {
  switch (kind) {
    case "shell":
      return "Shell";
    case "ssh":
      return "SSH";
    case "tmux":
      return "Tmux";
    case "telnet":
      return "Telnet";
    case "tcp":
      return "Tcp";
    case "serial":
      return "Serial";
  }
}

function convertDraftProtocol(draft: SessionProfile, protocol: ProtocolTab): SessionProfile {
  switch (protocol) {
    case "Shell":
      return { ...draft, kind: "shell", connection: draft.connection.kind === "shell" ? draft.connection : createShellConnection() };
    case "SSH":
      return { ...draft, kind: "ssh", connection: draft.connection.kind === "ssh" || draft.connection.kind === "tmux" ? { ...draft.connection, kind: "ssh" } : createSshConnection() };
    case "Tmux":
      return { ...draft, kind: "tmux", connection: draft.connection.kind === "ssh" || draft.connection.kind === "tmux" ? { ...draft.connection, kind: "tmux" } : { ...createSshConnection(), kind: "tmux" } };
    case "Telnet":
      return { ...draft, kind: "telnet", connection: draft.connection.kind === "telnet" ? draft.connection : createTcpConnection("telnet") };
    case "Tcp":
      return { ...draft, kind: "tcp", connection: draft.connection.kind === "tcp" ? draft.connection : createTcpConnection("tcp") };
    case "Serial":
      return { ...draft, kind: "serial", connection: draft.connection.kind === "serial" ? draft.connection : createSerialConnection() };
  }
}

function patchDraftSerial(draft: SessionProfile, onDraftChange: (draft: SessionProfile) => void, patch: Record<string, unknown>) {
  if (draft.connection.kind !== "serial") return;
  onDraftChange({ ...draft, connection: { ...draft.connection, ...patch } });
}

function describeEndpoint(session: SessionSummary) {
  return describeProfileEndpoint(session.profile);
}

function describeProfileEndpoint(profile: SessionProfile) {
  const connection = profile.connection;
  switch (connection.kind) {
    case "ssh":
    case "tmux":
      return connection.username ? `${connection.username}@${connection.endpoint.host}:${connection.endpoint.port}` : `${connection.endpoint.host}:${connection.endpoint.port}`;
    case "serial":
      return connection.port || "serial";
    case "shell":
      return connection.program || "shell";
    case "telnet":
    case "tcp":
      return `${connection.host}:${connection.port}`;
  }
}

function describeHostKeyEvaluation(result: HostKeyScanResult) {
  const evaluation = result.evaluation;
  const prefix = result.label ? `${result.label}: ` : "";
  if (evaluation.status === "trusted") {
    return `${prefix}已信任 ${evaluation.fingerprintSha256}`;
  }
  if (evaluation.status === "mismatch") {
    return `${prefix}不匹配 ${evaluation.algorithm} ${evaluation.observedFingerprintSha256}`;
  }
  return `${prefix}未知 ${evaluation.algorithm} ${evaluation.fingerprintSha256}`;
}

function defaultLocalPath() {
  return "/";
}

function joinFilePath(base: string, name: string, remote: boolean) {
  const separator = remote || base.includes("/") ? "/" : "\\";
  const cleanBase = base.endsWith("/") || base.endsWith("\\") ? base.slice(0, -1) : base;
  return cleanBase ? `${cleanBase}${separator}${name}` : name;
}

function filePaneAtPhysicalPosition(x: number, y: number): boolean | null {
  const scale = window.devicePixelRatio || 1;
  const target = document.elementFromPoint(x / scale, y / scale);
  const pane = target?.closest<HTMLElement>("[data-file-pane]");
  if (pane?.dataset.filePane === "remote") return true;
  if (pane?.dataset.filePane === "local") return false;
  return null;
}

function parentPath(path: string, remote: boolean) {
  const separator = remote || path.includes("/") ? "/" : "\\";
  const trimmed = path.replace(/[\\/]$/, "");
  const index = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  if (index <= 0) return separator === "/" ? "/" : trimmed;
  return trimmed.slice(0, index);
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MiB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GiB`;
}

function formatFileMode(mode?: number | null) {
  if (mode == null) return "-";
  return `0${(mode & 0o7777).toString(8).padStart(3, "0")}`;
}

function formatFileKind(properties: FileProperties) {
  if (properties.isSymlink) return "symlink";
  if (properties.isDir) return "directory";
  if (properties.isFile) return "file";
  return properties.kind || "other";
}

function formatDateTime(value?: string | null) {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

function formatDuration(start: string, end: string) {
  const elapsedMs = Math.max(0, Date.parse(end) - Date.parse(start));
  if (!Number.isFinite(elapsedMs)) return "";
  if (elapsedMs < 1000) return `${elapsedMs} ms`;
  const seconds = elapsedMs / 1000;
  if (seconds < 60) return `${seconds.toFixed(1)} s`;
  const minutes = Math.floor(seconds / 60);
  return `${minutes}m ${Math.round(seconds % 60)}s`;
}

function formatEventClock(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "--:--:--";
  return date.toLocaleTimeString([], { hour12: false, hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

function textToHex(value: string) {
  return Array.from(new TextEncoder().encode(value))
    .slice(0, 96)
    .map((byte) => byte.toString(16).padStart(2, "0").toUpperCase())
    .join(" ");
}

function formatSerialPreview(value: string) {
  const preview = value
    .replace(/\r/g, "\\r")
    .replace(/\n/g, "\\n")
    .replace(/\t/g, "\\t")
    .replace(/[^\x20-\x7e]/g, ".");
  return preview.length > 120 ? `${preview.slice(0, 120)}...` : preview;
}

function parseHexBytes(value: string) {
  const clean = value.replace(/0x/gi, "").replace(/[^0-9a-f]/gi, "");
  if (!clean) return [];
  const even = clean.length % 2 === 0 ? clean : `0${clean}`;
  return even.match(/.{1,2}/g)?.map((pair) => Number.parseInt(pair, 16)) ?? [];
}

function formatHexBytes(bytes: number[]) {
  return bytes.map((byte) => byte.toString(16).padStart(2, "0").toUpperCase()).join(" ");
}

function resolveSendTargets(target: SendTarget, activeId: string, sessions: SessionSummary[], panes: SessionSummary[]) {
  if (target === "connected") {
    return sessions.filter((session) => session.runtime.status === "connected").map((session) => session.profile.id);
  }
  if (target === "panes") {
    return panes.filter((session) => session.runtime.status === "connected").map((session) => session.profile.id);
  }
  return activeId ? [activeId] : [];
}

function delay(ms: number) {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}
