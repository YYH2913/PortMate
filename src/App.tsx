import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties, DragEvent as ReactDragEvent, FormEvent, MouseEvent as ReactMouseEvent, PointerEvent as ReactPointerEvent } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import {
  Activity,
  AlertCircle,
  Archive,
  ArrowRightLeft,
  ArrowUp,
  Ban,
  Check,
  CheckCircle2,
  Clock3,
  Copy,
  Download,
  File,
  FileText,
  Files,
  Folder,
  FolderPlus,
  Info,
  KeyRound,
  ListChecks,
  Lock,
  LoaderCircle,
  Maximize2,
  Minimize2,
  Package,
  PanelBottom,
  PanelLeft,
  PanelRight,
  Pencil,
  Play,
  Plus,
  RefreshCw,
  RotateCcw,
  Search,
  SendHorizontal,
  Settings,
  ShieldCheck,
  Square,
  SquareTerminal,
  Trash2,
  Unlock,
  Upload,
  UserPlus,
  X,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { callBackend, emptyAudit, emptyGrants, emptyHostKeys, emptyLogs, emptySessions, emptyTransfers, invokeBackend, isBackendAvailable } from "./api";
import type { CommandHistoryEntry } from "./command-history-state";
import { mergeTransfers } from "./transfer-state";
import { updateFileSelection } from "./file-selection";
import { KeyedRequestGate } from "./keyed-request-gate";
import { filterLogShards, selectVisibleLogShards, summarizeBundleAttachmentSelection } from "./log-shard-state";
import { MCP_APPROVAL_EVENT, mergeMcpApprovals } from "./mcp-approval-state";
import { menuGroups, menuItemDisabled } from "./menu-capabilities";
import type { MenuCapabilityContext, MenuItem } from "./menu-capabilities";
import { buildDetachedPanePath, DETACHED_PANE_EVENT, normalizeDetachedPaneCommand, normalizeDetachedPaneMessage } from "./detached-pane-state";
import type { DetachedPaneCommand, DetachedPaneRequest } from "./detached-pane-state";
import { normalizeProxyConfig, proxyDefaults } from "./proxy-settings";
import type { ProxyPasswordUpdate } from "./proxy-settings";
import { normalizeQuickCommandLibrary, QUICK_BAR_VISIBLE_STORAGE_KEY, QUICK_COMMAND_STORAGE_KEY, quickCommandDispatch } from "./quick-command-state";
import type { QuickCommand } from "./quick-command-state";
import { normalizeSerialConnectionSettings, serialConnectionBounds, serialConnectionDefaults } from "./serial-connection-settings";
import type { SerialAnalyzerRequest } from "./serial-analyzer-route";
import type { SearchDialogState } from "./SearchDialog";
import { flattenSessionTree, MAX_SESSION_PROFILE_GROUP_CHARACTERS, MAX_SESSION_PROFILE_NAME_CHARACTERS, MAX_SESSION_PROFILE_TAG_INPUT_CHARACTERS, normalizeSessionMetadataText, normalizeSessionProfileMetadata, protocolTabs, sessionSettingTrees } from "./session-settings-state";
import type { ProtocolTab } from "./session-settings-state";
import { sessionConnectionAction } from "./session-runtime-state";
import { filterSerialCaptureFrames, mergeSerialCaptureSnapshot, serialCaptureAscii, serialCaptureHex } from "./serial-capture-state";
import type { SerialCaptureDirectionFilter } from "./serial-capture-state";
import { createScreenLockMarker, decodeStoredScreenLockMarker, isScreenLockShortcut, MAX_SCREEN_LOCK_TIMEOUT_MINUTES, MIN_SCREEN_LOCK_TIMEOUT_MINUTES, normalizeScreenLockTimeoutMinutes, SCREEN_LOCK_STORAGE_KEY, shouldAutoLockScreen } from "./screen-lock-state";
import type { ScreenLockReason } from "./screen-lock-state";
import { normalizeSshConnectionSettings, sshConnectionBounds, sshConnectionDefaults } from "./ssh-connection-settings";
import { allSyncProtocols, defaultSyncInputSettings, normalizeSyncInputSettings, resolveSyncInputTargets, SyncInputDispatcher } from "./sync-input-state";
import type { SyncInputOrigin, SyncInputSettings, SyncNewlineMode } from "./sync-input-state";
import { requestTerminalFreeInput } from "./terminal-free-input";
import { requestTerminalTextExport } from "./terminal-export-event";
import type { TerminalTextExportSource } from "./terminal-export-event";
import type { TerminalBufferAction } from "./terminal-buffer-event";
import type { TerminalSelectionAction } from "./terminal-selection-event";
import { MAX_TERMINAL_FONT_FAMILY_CHARACTERS, MAX_TERMINAL_NAME_BYTES, normalizeTerminalProfileSettings, normalizeTerminalStartupSessionIds, TERMINAL_PROFILE_BOUNDS, terminalStartupSessionOptions } from "./terminal-settings-state";
import { requestTerminalGotoLine } from "./terminal-goto-line-event";
import { terminalKeyModeLabel, toggleTerminalRemoteLocalMode } from "./terminal-key-mode";
import type { TerminalKeyMode } from "./terminal-key-mode";
import { requestTerminalSearch } from "./terminal-search";
import { normalizeTerminalTheme, TERMINAL_THEME_OPTIONS } from "./terminal-theme";
import { transferDiagnosticText, transferDisplayMessage, transferStatusLabel } from "./transfer-presentation";
import { transferProtocolLabel, transferProtocolsForProfile } from "./transfer-capabilities";
import type { TransferProtocol } from "./transfer-capabilities";
import { defaultTriggerAction, patchTriggerAction, triggerActionValue } from "./trigger-state";
import { normalizeTcpConnectionSettings, tcpConnectionBounds, tcpConnectionDefaults } from "./tcp-connection-settings";
import { mergeSysmonHistory, normalizeSysmonHistory, sysmonTrendMax, sysmonTrendValue } from "./sysmon-history";
import type { SysmonTrendMode } from "./sysmon-history";
import { defaultWorkspaceKeymap, formatWorkspaceKeyBinding, LEGACY_WORKSPACE_KEYMAP_STORAGE_KEY, normalizeWorkspaceKeymap, resolveWorkspaceHotkeySequence, WORKSPACE_KEY_CHORD_TIMEOUT_MS, WORKSPACE_KEYMAP_STORAGE_KEY, workspaceHotkeyCommands, workspaceKeyBindingFromEvent, workspaceKeymapConflicts } from "./workspace-hotkeys";
import type { WorkspaceHotkeyCommandId, WorkspaceKeymap } from "./workspace-hotkeys";
import type { WorkspaceViewContextAction } from "./WorkspaceViewContextMenu";
import { activateWorkspaceDockPanel, activeWorkspaceDockPanel, clampWorkspaceDockSize, isWorkspaceFocusModeShortcut, LEGACY_WORKSPACE_PANEL_STORAGE_KEY, moveWorkspacePanelToDock, normalizeWorkspaceDockLayout, normalizeWorkspaceDockSizes, normalizeWorkspacePanelVisibility, resolveWorkspacePanelVisibility, setWorkspaceDockSize, setWorkspacePanelVisibility, visibleWorkspaceDockPanels, workspaceDockEffectiveSize, workspaceDockIds, workspaceDockPanelIds, workspaceDockSizeLimits, WORKSPACE_PANEL_STORAGE_KEY } from "./workspace-panel-state";
import type { WorkspaceDockId, WorkspaceDockLayout, WorkspaceDockPanelId, WorkspaceDockSizes, WorkspacePanelId } from "./workspace-panel-state";
import { workspaceSplitDirectionForVisualOrientation, workspaceViewContextCapabilities } from "./workspace-view-context-state";
import { activateWorkspacePaneSession, activateWorkspacePaneView, addWorkspacePaneSession, canSplitWorkspacePane, createWorkspaceNodeId, createWorkspacePane, createWorkspacePaneFromViews, duplicateWorkspacePaneView, findWorkspacePane, findWorkspacePaneBySession, findWorkspacePaneInDirection, insertWorkspacePaneView, MAX_WORKSPACE_DEPTH, MAX_WORKSPACE_GROUP_TABS, MAX_WORKSPACE_PANES, MAX_WORKSPACE_SPLIT_RATIO, mergeWorkspacePaneGroups, MIN_WORKSPACE_SPLIT_RATIO, moveWorkspacePaneView, moveWorkspacePaneViewToNewGroup, reconcileWorkspaceSnapshot, removeWorkspacePane, removeWorkspacePaneView, renameWorkspacePaneView, replaceWorkspacePaneSession, replaceWorkspacePaneView, resolveStartupSessionIds, sanitizeWorkspaceSnapshot, setWorkspacePaneViewColor, setWorkspacePaneViewKeyMode, splitWorkspacePane, splitWorkspacePaneViewToGroup, splitWorkspacePaneWithView, swapWorkspacePanes, updateWorkspaceSplitRatio, workspacePaneActiveView, workspacePaneLeaves, workspacePaneViewAtOffset } from "./workspace-state";
import type { StartupMode, WorkspaceNode, WorkspacePaneDirection, WorkspaceSnapshot, WorkspaceSplitDirection, WorkspaceSplitNode, WorkspaceSplitPlacement, WorkspaceView } from "./workspace-state";
import { buildProfileSecretMigrationRequest, canExecuteProfileSecretMigration, canRecoverProfileSecretMigration, exportProfileSecretMigrationDiagnostics, getProfileSecretMigrationRecovery, isProfileSecretMigrationRestartRequired, profileSecretMigrationErrorMessage, recoverProfileSecretMigration, sameProfileSecretMigrationRequest, summarizeProfileSecretCleanup } from "./secret-migration-state";
import type { ProfileSecretMigrationDiagnosticExportResult, ProfileSecretMigrationPreview, ProfileSecretMigrationRecoverySummary, ProfileSecretMigrationRequest, ProfileSecretMigrationResponse, SecretStorage } from "./secret-migration-state";
import type { ArchiveLogShardsResult, AuditRecord, AuthMethod, ConnectionConfig, DeleteLogShardsResult, DeleteSessionProfileResponse, ExportSerialCaptureResult, ExportSessionBundleArchiveResult, ExportTerminalTextResult, ExternalDropResult, FileEntry, FileProperties, HostKeyObservation, HostKeyPolicy, HostKeyScanResult, HostKeyStore, IdentityRef, JumpHop, LogShardInfo, LogShardPreview, LogShardSearchMatch, McpApprovalRequest, McpGrant, OneKeySummary, ProxyConfig, SearchLogShardsResult, SerialCaptureFrame, SerialCaptureSnapshot, SessionEvent, SessionKind, SessionProfile, SessionStatus, SessionSummary, SysmonSnapshot, TransferTask, TriggerAction, TriggerEffect, TriggerSpec, TunnelStatus, TunnelSpec, TrustedHostKey } from "./types";
import { selectedSshOneKey, sshOneKeysForSession } from "./one-key-login-state";
import type { OneKeyPromptField } from "./one-key-completion-state";
import { deleteSessionProfileFromClientState } from "./session-profile-delete-state";
import type { SessionContextAction, TerminalContextAction } from "./ContextMenus";

const LazyTerminalCanvas = lazy(() => import("./TerminalCanvas"));
const LazyQuickCommandDialog = lazy(() => import("./QuickCommandDialog"));
const LazyOneKeyDialog = lazy(() => import("./OneKeyDialog"));
const LazySearchDialog = lazy(() => import("./SearchDialog"));
const LazyTmuxDialog = lazy(() => import("./TmuxDialog"));
const LazyMcpDialog = lazy(() => import("./McpDialog"));
const LazyMcpApprovalDialog = lazy(() => import("./McpApprovalDialog"));
const LazySessionContextMenu = lazy(() => import("./ContextMenus").then(({ SessionContextMenu }) => ({ default: SessionContextMenu })));
const LazyTerminalContextMenu = lazy(() => import("./ContextMenus").then(({ TerminalContextMenu }) => ({ default: TerminalContextMenu })));
const LazySessionExplorerPanel = lazy(() => import("./WorkspaceUtilityPanels").then(({ SessionExplorerPanel }) => ({ default: SessionExplorerPanel })));
const LazyCommandHistoryList = lazy(() => import("./WorkspaceUtilityPanels").then(({ CommandHistoryList }) => ({ default: CommandHistoryList })));
const LazyWorkspaceViewContextMenu = lazy(() => import("./WorkspaceViewContextMenu"));
const LazyWorkspaceViewRenameDialog = lazy(() => import("./WorkspaceViewRenameDialog"));

const WORKSPACE_STORAGE_KEY = "portmate.workspace.v1";
const MAX_CLOSED_WORKSPACE_VIEWS = 32;
const COMMAND_HISTORY_STORAGE_KEY = "portmate.commandHistory";
const MAX_COMMAND_HISTORY_LIMIT = 10_000;
const MAX_COMMAND_HISTORY_RETENTION_DAYS = 3_650;
const MAX_RESOLVED_MCP_APPROVAL_IDS = 256;
const workspaceUtilityIcons = { Folder, Search, X };
const workspaceDockPanelMeta: Record<WorkspaceDockPanelId, { label: string; icon: LucideIcon }> = {
  explorer: { label: "资源管理器", icon: Folder },
  fileManager: { label: "文件管理器", icon: Files },
  history: { label: "历史命令", icon: Clock3 },
  sender: { label: "发送", icon: SendHorizontal },
};
const workspaceDockMeta: Record<WorkspaceDockId, { label: string; icon: LucideIcon }> = {
  left: { label: "左侧", icon: PanelLeft },
  right: { label: "右侧", icon: PanelRight },
  bottom: { label: "底部", icon: PanelBottom },
};

const workspacePanelMenuItems: Partial<Record<string, WorkspacePanelId>> = {
  资源管理器: "explorer",
  文件管理器: "fileManager",
  历史命令: "history",
  发送: "sender",
  状态栏: "statusBar",
};

function rememberResolvedMcpApproval(resolved: Set<string>, approvalId: string) {
  resolved.add(approvalId);
  while (resolved.size > MAX_RESOLVED_MCP_APPROVAL_IDS) {
    const oldest = resolved.values().next().value;
    if (!oldest) break;
    resolved.delete(oldest);
  }
}

const terminalKeyModeMenuItems: Partial<Record<string, TerminalKeyMode>> = {
  远程模式: "remote",
  本地模式: "local",
  "Normal 模式": "normal",
  "Command 模式": "command",
};

const terminalSettingPages = [
  "应用",
  "安全",
  "快捷键",
  "自动补全",
  "命令历史",
  "鼠标",
  "同步输入",
] as const;

const sessionKindLabels: Record<SessionKind, string> = {
  ssh: "SSH",
  tmux: "Tmux",
  serial: "Serial",
  shell: "Shell",
  telnet: "Telnet",
  tcp: "Raw TCP",
};

const migrationRecoveryStateLabels: Record<ProfileSecretMigrationRecoverySummary["state"], string> = {
  "target-write-pending": "目标写入待核对",
  "targets-verified": "目标已验证",
  "profiles-committed": "Profile 已提交",
  "source-cleanup-pending": "源清理待完成",
  "target-cleanup-pending": "目标回滚待完成",
  "needs-resolution": "需要人工核对",
};

const migrationRecoveryDispositionLabels: Record<ProfileSecretMigrationRecoverySummary["disposition"], string> = {
  "not-committed": "原引用生效",
  committed: "目标引用生效",
  conflict: "投影冲突",
};

type SettingsDialog = "terminal" | "session" | null;
type UtilityDialog = "transfer" | "tunnel" | "tmux" | "sysmon" | "search" | "logs" | "keys" | "mcp" | "one-keys" | "quick-commands" | null;
type TerminalPrefs = ReturnType<typeof createTerminalPrefs>;
type ConnectionCredentials = { username: string | null; password: string | null; passphrase: string | null; oneKeyId: string | null; savePassword: boolean; savePassphrase: boolean };
type NoticeState = { title: string; message: string } | null;
type WorkspaceGroupMoveRequest = { paneId: string; mode: "view" | "group" } | null;
type WorkspaceViewRenameRequest = { paneId: string; viewId: string; value: string; sessionName: string } | null;
type WorkspaceViewContextMenuState = { x: number; y: number; paneId: string; viewId: string } | null;
type ClosedWorkspaceView = { view: WorkspaceView; paneId: string; index: number };
type ScreenLockState = {
  reason: ScreenLockReason;
  lockedAt: number;
  mode: "preparing" | "vault" | "confirm" | "error";
  restoreVaultLocked: boolean | null;
  repairMarker: boolean;
  message: string;
} | null;
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
type ClientIdentityEditDraft = {
  profileId: string;
  identityId: string;
  label: string;
  source: IdentityRef["source"];
  fingerprintSha256: string;
  path: string;
  secretRef: string;
};
type ClientIdentityMutationResponse = {
  summary: SessionSummary;
  oldSecretDeleted: boolean;
  oldSecretShared: boolean;
  cleanupWarning?: string | null;
};
type PortableVaultStatus = {
  exists: boolean;
  unlocked: boolean;
  path: string;
};
type SecretStorageChoice = "auto" | "native" | "portable";
type HostKeyPromptState = {
  profile: SessionProfile;
  message: string;
  scan: HostKeyScanResult | null;
  scanError: string | null;
  busy: boolean;
};
type SendMode = "text" | "hex";
type SendTarget = "active" | "panes" | "connected";
type ContextMenuState = {
  kind: "session";
  x: number;
  y: number;
  sessionId: string | null;
} | {
  kind: "terminal";
  x: number;
  y: number;
  paneId: string;
  viewId: string;
  sessionId: string;
  alternate: boolean;
  hasSelection: boolean;
} | null;
type CredentialPromptState = {
  target: string;
  initialUsername: string;
  oneKeys: OneKeySummary[];
  hasIdentityFiles: boolean;
  hasSavedPassword: boolean;
  hasSavedPassphrase: boolean;
  needsPassword: boolean;
  authOrder: AuthMethod[];
};

const tabColorChoices = [
  { label: "深青", value: "#008B8B" },
  { label: "深粉", value: "#FF1493" },
  { label: "森林绿", value: "#228B22" },
  { label: "金菊", value: "#DAA520" },
  { label: "印度红", value: "#CD5C5C" },
  { label: "兰紫", value: "#BA55D3" },
  { label: "板岩蓝", value: "#7B68EE" },
  { label: "橄榄", value: "#808000" },
  { label: "红色", value: "#FF0000" },
  { label: "皇家蓝", value: "#4169E1" },
  { label: "钢蓝", value: "#4682B4" },
  { label: "水鸭", value: "#008080" },
];

export default function App() {
  const [initialWorkspace] = useState(loadWorkspaceSnapshot);
  const [terminalPrefs, setTerminalPrefs] = useState(loadTerminalPrefs);
  const [screenLock, setScreenLock] = useState<ScreenLockState>(() => loadInitialScreenLockState(terminalPrefs.requireMasterPassword));
  const [sessions, setSessions] = useState<SessionSummary[]>(emptySessions);
  const [logs, setLogs] = useState<Record<string, SessionEvent[]>>(emptyLogs);
  const [transfers, setTransfers] = useState<TransferTask[]>(emptyTransfers);
  const [audit, setAudit] = useState<AuditRecord[]>(emptyAudit);
  const [grants, setGrants] = useState<McpGrant[]>(emptyGrants);
  const [mcpApprovals, setMcpApprovals] = useState<McpApprovalRequest[]>([]);
  const [hostKeys, setHostKeys] = useState<HostKeyStore>(emptyHostKeys);
  const [oneKeys, setOneKeys] = useState<OneKeySummary[]>([]);
  const [serialPorts, setSerialPorts] = useState<string[]>([]);
  const [serialCaptures, setSerialCaptures] = useState<Record<string, SerialCaptureFrame[]>>({});
  const [activeId, setActiveId] = useState(initialWorkspace.activeId);
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
  const [syncInput, setSyncInput] = useState(false);
  const [syncInputSettings, setSyncInputSettings] = useState<SyncInputSettings>(() => (
    normalizeSyncInputSettings(loadLocalValue<unknown>("portmate.syncInputSettings", defaultSyncInputSettings))
  ));
  const commandHistoryPolicy = useMemo(
    () => ({
      limit: normalizeCommandHistoryInteger(terminalPrefs.historyLimit, 1, MAX_COMMAND_HISTORY_LIMIT, MAX_COMMAND_HISTORY_LIMIT),
      retentionDays: normalizeCommandHistoryInteger(terminalPrefs.historyRetentionDays, 0, MAX_COMMAND_HISTORY_RETENTION_DAYS, 30),
    }),
    [terminalPrefs.historyLimit, terminalPrefs.historyRetentionDays],
  );
  const [commandHistoryEntries, setCommandHistoryEntries] = useState<CommandHistoryEntry[]>([]);
  const [commandHistoryReady, setCommandHistoryReady] = useState(false);
  const commandHistory = commandHistoryEntries.map((entry) => entry.command);
  const [quickCommands, setQuickCommands] = useState<QuickCommand[]>(() => (
    normalizeQuickCommandLibrary(loadLocalValue<unknown>(QUICK_COMMAND_STORAGE_KEY, null)).items
  ));
  const [quickBarVisible, setQuickBarVisible] = useState(() => (
    loadLocalValue<unknown>(QUICK_BAR_VISIBLE_STORAGE_KEY, false) === true
  ));
  const [workspacePanels, setWorkspacePanels] = useState(() => (
    normalizeWorkspacePanelVisibility(loadLocalValue<unknown>(
      WORKSPACE_PANEL_STORAGE_KEY,
      loadLocalValue<unknown>(LEGACY_WORKSPACE_PANEL_STORAGE_KEY, null),
    ))
  ));
  const [workspaceDockLayout, setWorkspaceDockLayout] = useState<WorkspaceDockLayout>(() => (
    normalizeWorkspaceDockLayout(loadLocalValue<unknown>(WORKSPACE_PANEL_STORAGE_KEY, null))
  ));
  const [workspaceDockSizes, setWorkspaceDockSizes] = useState<WorkspaceDockSizes>(() => (
    normalizeWorkspaceDockSizes(loadLocalValue<unknown>(WORKSPACE_PANEL_STORAGE_KEY, null))
  ));
  const [draggedWorkspacePanel, setDraggedWorkspacePanel] = useState<WorkspaceDockPanelId | null>(null);
  const [focusMode, setFocusMode] = useState(false);
  const [notice, setNotice] = useState<NoticeState>(null);
  const [hostKeyPrompt, setHostKeyPrompt] = useState<HostKeyPromptState | null>(null);
  const [sessionSettingsSection, setSessionSettingsSection] = useState("会话");
  const [credentialPrompt, setCredentialPrompt] = useState<CredentialPromptState | null>(null);
  const [workspaceRoot, setWorkspaceRoot] = useState<WorkspaceNode | null>(initialWorkspace.root);
  const [activePaneId, setActivePaneId] = useState(initialWorkspace.activePaneId);
  const [zoomedPaneId, setZoomedPaneId] = useState("");
  const [workspaceGroupMove, setWorkspaceGroupMove] = useState<WorkspaceGroupMoveRequest>(null);
  const [workspaceViewRename, setWorkspaceViewRename] = useState<WorkspaceViewRenameRequest>(null);
  const [workspaceViewContextMenu, setWorkspaceViewContextMenu] = useState<WorkspaceViewContextMenuState>(null);
  const [closedWorkspaceViews, setClosedWorkspaceViews] = useState<ClosedWorkspaceView[]>([]);
  const [workspaceKeymap, setWorkspaceKeymap] = useState<WorkspaceKeymap>(() => (
    normalizeWorkspaceKeymap(loadLocalValue<unknown>(
      WORKSPACE_KEYMAP_STORAGE_KEY,
      loadLocalValue<unknown>(LEGACY_WORKSPACE_KEYMAP_STORAGE_KEY, defaultWorkspaceKeymap),
    ))
  ));
  const [blockSelection, setBlockSelection] = useState(false);
  const [contextMenu, setContextMenu] = useState<ContextMenuState>(null);
  const [tabColors, setTabColors] = useState<Record<string, string>>(initialWorkspace.tabColors);
  const credentialResolverRef = useRef<((credentials: ConnectionCredentials | null) => void) | null>(null);
  const startupAppliedRef = useRef(false);
  const syncInputDispatcherRef = useRef(new SyncInputDispatcher());
  const syncInputRef = useRef(false);
  const logSignatureRef = useRef<Record<string, string>>({});
  const activeLogRefreshGateRef = useRef(new KeyedRequestGate<string>());
  const sessionsSignatureRef = useRef("");
  const serialCapturesRef = useRef<Record<string, SerialCaptureFrame[]>>({});
  const serialCaptureRefreshesRef = useRef(new Set<string>());
  const serialCaptureEpochRef = useRef<Record<string, number>>({});
  const resolvedMcpApprovalsRef = useRef(new Set<string>());
  const detachedCommandHandlerRef = useRef<(command: DetachedPaneCommand) => void>(() => {});
  const screenLockRef = useRef<ScreenLockState>(screenLock);
  const restoredScreenLockPreparedRef = useRef(false);

  const active = sessions.find((session) => session.profile.id === activeId);
  const activeMcpApproval = mcpApprovals[0];
  const approvalSessionName = sessions.find((session) => session.profile.id === activeMcpApproval?.sessionId)?.profile.name ?? activeMcpApproval?.sessionId ?? "";
  const activeWorkspacePane = findWorkspacePane(workspaceRoot, activePaneId);
  const activeWorkspaceView = activeWorkspacePane ? workspacePaneActiveView(activeWorkspacePane) : undefined;
  const activeTerminalKeyMode = activeWorkspaceView?.keyMode ?? "remote";
  const workspaceContextPane = workspaceViewContextMenu
    ? findWorkspacePane(workspaceRoot, workspaceViewContextMenu.paneId)
    : undefined;
  const workspaceContextView = workspaceContextPane?.views.find((view) => view.id === workspaceViewContextMenu?.viewId);
  const workspaceContextSession = sessions.find((session) => session.profile.id === workspaceContextView?.sessionId);
  const workspacePanes = workspacePaneLeaves(workspaceRoot);
  const workspaceContextCapabilities = workspaceViewContextCapabilities(
    workspaceContextPane,
    workspaceContextView?.id,
    workspacePanes.length,
    workspacePanes.reduce((count, pane) => count + pane.views.length, 0),
    closedWorkspaceViews.some((closed) => sessions.some((session) => session.profile.id === closed.view.sessionId)),
    workspacePanes.length < MAX_WORKSPACE_PANES
      && canSplitWorkspacePane(workspaceRoot, workspaceContextPane?.id ?? ""),
  );
  const workspaceContextCanMerge = Boolean(workspaceContextPane && workspacePanes.some((pane) => (
    pane.id !== workspaceContextPane.id
      && pane.views.length + workspaceContextPane.views.length <= MAX_WORKSPACE_GROUP_TABS
  )));
  const workspaceContextCanSwap: Record<WorkspacePaneDirection, boolean> = {
    up: Boolean(workspaceContextPane && findWorkspacePaneInDirection(workspaceRoot, workspaceContextPane.id, "up")),
    down: Boolean(workspaceContextPane && findWorkspacePaneInDirection(workspaceRoot, workspaceContextPane.id, "down")),
    left: Boolean(workspaceContextPane && findWorkspacePaneInDirection(workspaceRoot, workspaceContextPane.id, "left")),
    right: Boolean(workspaceContextPane && findWorkspacePaneInDirection(workspaceRoot, workspaceContextPane.id, "right")),
  };
  const activeStatus = active?.runtime.status;
  const activeSerial = active?.profile.connection.kind === "serial" ? active.profile.connection : null;
  const menuCapabilityContext: MenuCapabilityContext = {
    hasActiveSession: Boolean(active),
    hasActiveView: Boolean(activeWorkspaceView),
    activeKind: active?.profile.kind ?? null,
    activeStatus: active?.runtime.status ?? null,
  };
  syncInputRef.current = syncInput;
  screenLockRef.current = screenLock;
  detachedCommandHandlerRef.current = (command) => {
    if (command.action === "lock-screen") {
      lockScreen("manual");
    } else if (command.action === "connect") {
      void connectSession(command.sessionId, undefined, false);
    } else if (command.action === "disconnect") {
      void disconnectSession(command.sessionId, false);
    } else {
      reattachDetachedPane(command);
    }
  };

  function updateSyncInput(enabled: boolean) {
    if (!enabled && syncInputRef.current) {
      syncInputDispatcherRef.current.cancelBroadcasts();
    }
    syncInputRef.current = enabled;
    setSyncInput(enabled);
  }

  function commitScreenLock(next: ScreenLockState) {
    screenLockRef.current = next;
    setScreenLock(next);
  }

  function clearScreenLock() {
    try {
      window.localStorage.removeItem(SCREEN_LOCK_STORAGE_KEY);
    } catch {
      // The in-memory lock still clears when local storage is unavailable.
    }
    clearScreenLockVaultRestoreState();
    commitScreenLock(null);
  }

  async function prepareScreenLock(state: NonNullable<ScreenLockState>) {
    const preparing = { ...state, mode: "preparing" as const, message: "" };
    let restoreVaultLocked = state.restoreVaultLocked;
    commitScreenLock(preparing);
    if (!isBackendAvailable()) {
      if (screenLockRef.current?.lockedAt === state.lockedAt) {
        commitScreenLock({
          ...preparing,
          mode: "confirm",
          message: "浏览器预览未连接桌面凭据库",
        });
      }
      return;
    }
    try {
      const vault = await invokeBackend<PortableVaultStatus>("portable_vault_status", {});
      if (screenLockRef.current?.lockedAt !== state.lockedAt) return;
      if (!vault.exists) {
        clearScreenLockVaultRestoreState();
        commitScreenLock({
          ...preparing,
          mode: "confirm",
          message: "尚未配置 Portable Vault 主密码",
        });
        return;
      }
      if (restoreVaultLocked === null) restoreVaultLocked = !vault.unlocked;
      saveScreenLockVaultRestoreState(restoreVaultLocked);
      if (vault.unlocked) {
        await invokeBackend<PortableVaultStatus>("lock_portable_vault", {});
      }
      if (screenLockRef.current?.lockedAt !== state.lockedAt) return;
      commitScreenLock({
        ...preparing,
        mode: "vault",
        restoreVaultLocked,
        message: "Portable Vault 已锁定",
      });
    } catch {
      if (screenLockRef.current?.lockedAt !== state.lockedAt) return;
      commitScreenLock({
        ...preparing,
        mode: "error",
        restoreVaultLocked,
        message: "无法确认 Portable Vault 状态",
      });
    }
  }

  function lockScreen(reason: Exclude<ScreenLockReason, "restored"> = "manual") {
    if (screenLockRef.current) return;
    clearScreenLockVaultRestoreState();
    const marker = createScreenLockMarker(reason);
    try {
      window.localStorage.setItem(SCREEN_LOCK_STORAGE_KEY, JSON.stringify(marker));
    } catch {
      // The current render remains locked even if reload persistence is unavailable.
    }
    const next: NonNullable<ScreenLockState> = {
      reason,
      lockedAt: marker.lockedAt,
      mode: "preparing",
      restoreVaultLocked: null,
      repairMarker: false,
      message: "",
    };
    setOpenMenu(null);
    setContextMenu(null);
    setWorkspaceViewContextMenu(null);
    window.getSelection()?.removeAllRanges();
    commitScreenLock(next);
    void prepareScreenLock(next);
  }

  async function unlockScreen(password = "") {
    const current = screenLockRef.current;
    if (!current) return;
    if (current.mode === "confirm") {
      clearScreenLock();
      return;
    }
    if (current.mode !== "vault" || !password) {
      throw new Error("请输入 Portable Vault 主密码");
    }
    let unlocked: PortableVaultStatus;
    try {
      unlocked = await invokeBackend<PortableVaultStatus>("unlock_portable_vault", {
        request: { password },
      });
    } catch {
      throw new Error("主密码验证失败");
    }
    if (!unlocked.unlocked) throw new Error("Portable Vault 未解锁");
    if (current.restoreVaultLocked) {
      try {
        await invokeBackend<PortableVaultStatus>("lock_portable_vault", {});
      } catch {
        throw new Error("凭据锁定状态恢复失败，请重试");
      }
    }
    if (screenLockRef.current?.lockedAt === current.lockedAt) clearScreenLock();
  }

  function retryPrepareScreenLock() {
    const current = screenLockRef.current;
    if (current) void prepareScreenLock(current);
  }

  useEffect(() => {
    void refresh();
  }, []);

  useEffect(() => {
    if (restoredScreenLockPreparedRef.current) return;
    restoredScreenLockPreparedRef.current = true;
    const current = screenLockRef.current;
    if (current?.mode === "preparing") {
      try {
        if (current.reason !== "restored" || current.repairMarker) {
          const reason = current.reason === "restored" ? "manual" : current.reason;
          const marker = createScreenLockMarker(reason, current.lockedAt);
          window.localStorage.setItem(SCREEN_LOCK_STORAGE_KEY, JSON.stringify(marker));
        }
      } catch {
        // The opaque first render remains locked even if persistence is unavailable.
      }
      void prepareScreenLock(current);
    }
  }, []);

  useEffect(() => {
    const handleScreenLockShortcut = (event: KeyboardEvent) => {
      if (!isScreenLockShortcut(event)) return;
      event.preventDefault();
      event.stopPropagation();
      if (!screenLockRef.current) lockScreen("manual");
    };
    window.addEventListener("keydown", handleScreenLockShortcut, true);
    return () => window.removeEventListener("keydown", handleScreenLockShortcut, true);
  }, []);

  useEffect(() => {
    if (!terminalPrefs.lockOnIdle || screenLock) return;
    const timeoutMinutes = normalizeScreenLockTimeoutMinutes(terminalPrefs.lockScreenTimeoutMinutes);
    let lastActivityAt = Date.now();
    let lastPointerActivityAt = 0;
    let timer: number | undefined;

    const checkDeadline = () => {
      if (shouldAutoLockScreen(true, lastActivityAt, Date.now(), timeoutMinutes)) {
        lockScreen("idle");
        return;
      }
      armTimer();
    };
    const armTimer = () => {
      if (timer !== undefined) window.clearTimeout(timer);
      const elapsed = Math.max(0, Date.now() - lastActivityAt);
      timer = window.setTimeout(checkDeadline, Math.max(0, timeoutMinutes * 60_000 - elapsed));
    };
    const recordActivity = (event: Event) => {
      if (document.visibilityState !== "visible") return;
      const now = Date.now();
      if (event.type === "pointermove" && now - lastPointerActivityAt < 250) return;
      if (event.type === "pointermove") lastPointerActivityAt = now;
      lastActivityAt = now;
      armTimer();
    };
    const handleVisibility = () => {
      if (document.visibilityState === "visible") checkDeadline();
    };
    const activityEvents = ["keydown", "pointerdown", "pointermove", "touchstart", "wheel"] as const;
    activityEvents.forEach((eventName) => window.addEventListener(eventName, recordActivity, { capture: true, passive: true }));
    document.addEventListener("visibilitychange", handleVisibility);
    armTimer();
    return () => {
      if (timer !== undefined) window.clearTimeout(timer);
      activityEvents.forEach((eventName) => window.removeEventListener(eventName, recordActivity, true));
      document.removeEventListener("visibilitychange", handleVisibility);
    };
  }, [screenLock, terminalPrefs.lockOnIdle, terminalPrefs.lockScreenTimeoutMinutes]);

  useEffect(() => {
    if (startupAppliedRef.current || !sessions.length) return;
    startupAppliedRef.current = true;
    const prefs = terminalPrefs;
    const workspace = reconcileWorkspaceSnapshot({
      version: 4,
      root: workspaceRoot,
      activePaneId,
      activeId,
      tabColors,
    }, sessions.map((session) => session.profile.id));
    const mode: StartupMode = prefs.startupMode === "none" || prefs.startupMode === "specific"
      ? prefs.startupMode
      : "last";
    const targets = resolveStartupSessionIds(mode, prefs.startupSessions, workspace, sessions.map((session) => session.profile.id));
    void (async () => {
      for (const sessionId of targets) {
        const session = sessions.find((item) => item.profile.id === sessionId);
        if (session?.runtime.status === "disconnected") {
          await connectSession(sessionId);
        }
      }
    })();
  }, [sessions]);

  useEffect(() => {
    saveLocalValue<WorkspaceSnapshot>(WORKSPACE_STORAGE_KEY, {
      version: 4,
      root: workspaceRoot,
      activePaneId,
      activeId,
      tabColors,
    });
  }, [workspaceRoot, activePaneId, activeId, tabColors]);

  useEffect(() => {
    if (zoomedPaneId && !findWorkspacePane(workspaceRoot, zoomedPaneId)) setZoomedPaneId("");
  }, [workspaceRoot, zoomedPaneId]);

  useEffect(() => {
    saveLocalValue(WORKSPACE_KEYMAP_STORAGE_KEY, workspaceKeymap);
  }, [workspaceKeymap]);

  useEffect(() => {
    const handleWorkspaceHotkey = (event: KeyboardEvent) => {
      if (!isWorkspaceHotkeyTarget(event.target)) {
        clearWorkspaceChord();
        return;
      }
      if (isWorkspaceFocusModeShortcut(event)) {
        consumeWorkspaceHotkey(event);
        clearWorkspaceChord();
        if (!event.repeat) setFocusMode((current) => !current);
        return;
      }
      const panes = workspacePaneLeaves(workspaceRoot);
      const hadPendingChord = Boolean(chordPrefix);
      if (hadPendingChord && event.repeat) {
        consumeWorkspaceHotkey(event);
        return;
      }
      if (hadPendingChord && isPlainEscape(event)) {
        consumeWorkspaceHotkey(event);
        clearWorkspaceChord();
        return;
      }
      const resolution = resolveWorkspaceHotkeySequence(
        event,
        panes.length,
        workspaceKeymap,
        chordPrefix,
        { terminalKeyMode: activeTerminalKeyMode },
      );
      if (hadPendingChord && isModifierKeyEvent(event)) return;
      if (hadPendingChord) {
        consumeWorkspaceHotkey(event);
        clearWorkspaceChord();
      } else if (resolution.kind === "none") {
        return;
      } else {
        consumeWorkspaceHotkey(event);
      }
      if (resolution.kind === "pending") {
        chordPrefix = resolution.prefix;
        chordTimer = window.setTimeout(clearWorkspaceChord, WORKSPACE_KEY_CHORD_TIMEOUT_MS);
        return;
      }
      if (resolution.kind !== "action") return;
      const hotkey = resolution.action;
      if (hotkey.kind === "focus") {
        const nextPane = findWorkspacePaneInDirection(workspaceRoot, activePaneId, hotkey.direction);
        if (nextPane) activateWorkspacePane(nextPane.id, nextPane.activeViewId);
      } else if (hotkey.kind === "cycle-view") {
        cycleActiveWorkspaceView(hotkey.offset);
      } else if (!event.repeat && hotkey.kind === "split") {
        splitWorkspace(hotkey.direction, hotkey.placement);
      } else if (!event.repeat && hotkey.kind === "close") {
        closeWorkspacePane();
      } else if (!event.repeat && hotkey.kind === "view-history") {
        if (hotkey.operation === "close") closeActiveWorkspaceView();
        else reopenClosedWorkspaceView();
      } else if (!event.repeat && hotkey.kind === "zoom") {
        toggleWorkspaceZoom();
      } else if (!event.repeat && hotkey.kind === "one-keys") {
        setUtilityDialog("one-keys");
      }
    };
    let chordPrefix = "";
    let chordTimer: number | undefined;
    const clearWorkspaceChord = () => {
      chordPrefix = "";
      if (chordTimer !== undefined) window.clearTimeout(chordTimer);
      chordTimer = undefined;
    };
    window.addEventListener("keydown", handleWorkspaceHotkey, true);
    return () => {
      clearWorkspaceChord();
      window.removeEventListener("keydown", handleWorkspaceHotkey, true);
    };
  }, [activeId, activePaneId, activeTerminalKeyMode, sessions, workspaceKeymap, workspaceRoot]);

  useEffect(() => {
    saveLocalValue("portmate.syncInputSettings", syncInputSettings);
  }, [syncInputSettings]);

  useEffect(() => {
    let disposed = false;
    void import("./command-history-state").then(({ normalizeCommandHistory }) => {
      if (disposed) return;
      setCommandHistoryEntries(normalizeCommandHistory(
        terminalPrefs.historyEnabled
          ? loadLocalValue<unknown>(COMMAND_HISTORY_STORAGE_KEY, null)
          : null,
        commandHistoryPolicy,
      ));
      setCommandHistoryReady(true);
    });
    return () => { disposed = true; };
  }, []);

  useEffect(() => {
    if (!commandHistoryReady) return;
    let disposed = false;
    void import("./command-history-state").then((history) => {
      if (disposed) return;
      const normalized = history.normalizeCommandHistory(
        history.commandHistorySnapshot(commandHistoryEntries),
        commandHistoryPolicy,
      );
      if (!history.commandHistoryEntriesEqual(commandHistoryEntries, normalized)) {
        setCommandHistoryEntries(normalized);
        return;
      }
      try {
        if (terminalPrefs.historyEnabled && normalized.length) {
          window.localStorage.setItem(COMMAND_HISTORY_STORAGE_KEY, JSON.stringify(history.commandHistorySnapshot(normalized)));
        } else {
          window.localStorage.removeItem(COMMAND_HISTORY_STORAGE_KEY);
        }
      } catch {
        // History persistence is best-effort; the in-memory list remains available.
      }
    });
    return () => { disposed = true; };
  }, [commandHistoryEntries, commandHistoryPolicy, commandHistoryReady, terminalPrefs.historyEnabled]);

  useEffect(() => {
    saveLocalValue(QUICK_COMMAND_STORAGE_KEY, { version: 1, items: quickCommands });
  }, [quickCommands]);

  useEffect(() => {
    saveLocalValue(QUICK_BAR_VISIBLE_STORAGE_KEY, quickBarVisible);
  }, [quickBarVisible]);

  useEffect(() => {
    saveLocalValue(WORKSPACE_PANEL_STORAGE_KEY, {
      version: 6,
      panels: workspacePanels,
      docks: workspaceDockLayout,
      sizes: workspaceDockSizes,
    });
  }, [workspaceDockLayout, workspaceDockSizes, workspacePanels]);

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
    if (!activeId || !activeSerial || !isBackendAvailable()) return;
    void refreshSerialCapture(activeId);
    if (activeStatus === "disconnected") return;
    const timer = window.setInterval(() => void refreshSerialCapture(activeId), 750);
    return () => window.clearInterval(timer);
  }, [activeId, activeStatus, active?.profile.kind]);

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

  useEffect(() => {
    if (!isBackendAvailable()) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;
    const pruneTimer = window.setInterval(() => {
      setMcpApprovals((current) => mergeMcpApprovals(current, [], Date.now(), resolvedMcpApprovalsRef.current));
    }, 1000);
    void listen<unknown>(MCP_APPROVAL_EVENT, (event) => {
      if (disposed) return;
      setMcpApprovals((current) => mergeMcpApprovals(current, [event.payload], Date.now(), resolvedMcpApprovalsRef.current));
    }).then(async (nextUnlisten) => {
      if (disposed) {
        nextUnlisten();
        return;
      }
      unlisten = nextUnlisten;
      try {
        const pending = await invokeBackend<unknown[]>("list_mcp_approvals", {});
        if (!disposed) setMcpApprovals((current) => mergeMcpApprovals(current, pending, Date.now(), resolvedMcpApprovalsRef.current));
      } catch {
        // A later approval event can still populate the queue.
      }
    }).catch(() => {});
    return () => {
      disposed = true;
      window.clearInterval(pruneTimer);
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!isBackendAvailable()) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<TriggerEffect>("portmate-trigger-effect", (event) => {
      if (disposed) return;
      const effect = event.payload;
      if (effect.kind === "highlight") {
        if (/^#[0-9a-f]{6}$/i.test(effect.value)) {
          setTabColors((current) => ({ ...current, [effect.sessionId]: effect.value }));
        }
        return;
      }
      if (effect.kind === "sound") {
        void playTriggerSound(effect.value).catch(() => {});
        return;
      }
      setNotice({
        title: effect.kind === "custom-link" ? `触发链接 · ${effect.triggerLabel}` : effect.triggerLabel,
        message: effect.value,
      });
    })
      .then((nextUnlisten) => {
        if (disposed) nextUnlisten();
        else unlisten = nextUnlisten;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    const handleBrowserMessage = (event: MessageEvent) => {
      if (event.origin !== window.location.origin) return;
      const message = normalizeDetachedPaneMessage(event.data);
      if (message) detachedCommandHandlerRef.current(message.payload);
    };
    window.addEventListener("message", handleBrowserMessage);
    if (isBackendAvailable()) {
      void listen<unknown>(DETACHED_PANE_EVENT, (event) => {
        const command = normalizeDetachedPaneCommand(event.payload);
        if (command) detachedCommandHandlerRef.current(command);
      }).then((nextUnlisten) => {
        if (disposed) nextUnlisten();
        else unlisten = nextUnlisten;
      }).catch(() => {});
    }
    return () => {
      disposed = true;
      unlisten?.();
      window.removeEventListener("message", handleBrowserMessage);
    };
  }, []);

  async function respondMcpApproval(approvalId: string, approved: boolean) {
    await invokeBackend<void>("respond_mcp_approval", { approvalId, approved });
    rememberResolvedMcpApproval(resolvedMcpApprovalsRef.current, approvalId);
    setMcpApprovals((current) => mergeMcpApprovals(
      current.filter((request) => request.id !== approvalId),
      [],
      Date.now(),
      resolvedMcpApprovalsRef.current,
    ));
  }

  function expireMcpApproval(approvalId: string) {
    rememberResolvedMcpApproval(resolvedMcpApprovalsRef.current, approvalId);
    setMcpApprovals((current) => mergeMcpApprovals(
      current.filter((request) => request.id !== approvalId),
      [],
      Date.now(),
      resolvedMcpApprovalsRef.current,
    ));
  }

  async function refresh() {
    const nextSessions = await callBackend("list_sessions", {}, loadLocalSessionSummaries());
    setSessions(nextSessions);
    setTransfers(await callBackend("list_transfers", {}, emptyTransfers));
    setAudit(await callBackend("list_mcp_audit", {}, emptyAudit));
    setGrants(await callBackend("list_mcp_grants", {}, emptyGrants));
    setHostKeys(await callBackend("list_host_keys", {}, emptyHostKeys));
    setOneKeys(await callBackend("list_one_keys", {}, []));
    setSerialPorts(await callBackend("list_serial_ports", {}, []));
    const restored = reconcileWorkspaceSnapshot({
      version: 4,
      root: workspaceRoot,
      activePaneId,
      activeId,
      tabColors,
    }, nextSessions.map((session) => session.profile.id));
    setWorkspaceRoot(restored.root);
    setActivePaneId(restored.activePaneId);
    setActiveId(restored.activeId);
    setTabColors(restored.tabColors);

    const nextLogs: Record<string, SessionEvent[]> = {};
    for (const session of nextSessions) {
      nextLogs[session.profile.id] = await callBackend("tail_log", { sessionId: session.profile.id, limit: 160 }, []);
    }
    activeLogRefreshGateRef.current.invalidateAll();
    logSignatureRef.current = Object.fromEntries(
      Object.entries(nextLogs).map(([sessionId, events]) => [sessionId, logSignature(events)]),
    );
    setLogs(nextLogs);
  }

  async function refreshActiveLog(sessionId: string) {
    const gate = activeLogRefreshGateRef.current;
    const token = gate.begin(sessionId);
    if (token === null) return;
    try {
      const nextLog = await invokeBackend<SessionEvent[]>("tail_log", { sessionId, limit: 600 });
      if (!gate.isCurrent(sessionId, token)) return;
      const signature = logSignature(nextLog);
      if (logSignatureRef.current[sessionId] === signature) return;
      logSignatureRef.current[sessionId] = signature;
      setLogs((current) => ({ ...current, [sessionId]: nextLog }));
    } catch {
      // Polling failures retain the last valid log snapshot.
    } finally {
      gate.finish(sessionId, token);
    }
  }

  function replaceSessionLog(sessionId: string, nextLog: SessionEvent[]) {
    activeLogRefreshGateRef.current.invalidate(sessionId);
    logSignatureRef.current[sessionId] = logSignature(nextLog);
    setLogs((current) => ({ ...current, [sessionId]: nextLog }));
  }

  function storeSerialCapture(sessionId: string, frames: SerialCaptureFrame[]) {
    serialCapturesRef.current = { ...serialCapturesRef.current, [sessionId]: frames };
    setSerialCaptures((current) => ({ ...current, [sessionId]: frames }));
  }

  async function refreshSerialCapture(sessionId: string) {
    if (serialCaptureRefreshesRef.current.has(sessionId)) return;
    serialCaptureRefreshesRef.current.add(sessionId);
    const epoch = serialCaptureEpochRef.current[sessionId] ?? 0;
    try {
      const current = serialCapturesRef.current[sessionId] ?? [];
      const snapshot = await invokeBackend<SerialCaptureSnapshot>("list_serial_capture", {
        sessionId,
        afterId: current.at(-1)?.id ?? null,
      });
      if ((serialCaptureEpochRef.current[sessionId] ?? 0) !== epoch) return;
      storeSerialCapture(sessionId, mergeSerialCaptureSnapshot(current, snapshot));
    } catch {
      // Capture polling is best-effort; transport status and terminal output remain authoritative.
    } finally {
      serialCaptureRefreshesRef.current.delete(sessionId);
    }
  }

  function appendLocalSerialCapture(sessionId: string, bytes: number[]) {
    const captured = bytes.slice(0, 64 * 1024);
    const frame: SerialCaptureFrame = {
      id: globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random()}`,
      ts: new Date().toISOString(),
      direction: "outbound",
      bytes: captured,
      originalLength: bytes.length,
      truncated: captured.length !== bytes.length,
    };
    const next = [...(serialCapturesRef.current[sessionId] ?? []), frame].slice(-512);
    while (next.reduce((total, item) => total + item.bytes.length, 0) > 1024 * 1024) {
      next.shift();
    }
    storeSerialCapture(sessionId, next);
  }

  async function clearSerialCapture(sessionId: string) {
    serialCaptureEpochRef.current = {
      ...serialCaptureEpochRef.current,
      [sessionId]: (serialCaptureEpochRef.current[sessionId] ?? 0) + 1,
    };
    if (!isBackendAvailable()) {
      storeSerialCapture(sessionId, []);
      return;
    }
    try {
      const snapshot = await invokeBackend<SerialCaptureSnapshot>("clear_serial_capture", { sessionId });
      storeSerialCapture(sessionId, mergeSerialCaptureSnapshot([], snapshot));
    } catch (error) {
      setNotice({ title: "清空串口捕获失败", message: formatError(error) });
    }
  }

  async function exportSerialCapture(sessionId: string, frameIds: string[]) {
    try {
      const result = await invokeBackend<ExportSerialCaptureResult>("export_serial_capture", {
        request: { sessionId, frameIds },
      });
      setNotice({
        title: "串口捕获已导出",
        message: `${result.frames} 帧 · ${formatBytes(result.capturedBytes)} · ${result.path}\nSHA-256 ${result.sha256}`,
      });
    } catch (error) {
      setNotice({ title: "导出串口捕获失败", message: formatError(error) });
    }
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
  const paneSessions = useMemo(() => {
    const ids = workspacePaneLeaves(workspaceRoot).map((pane) => workspacePaneActiveView(pane).sessionId);
    const resolvedIds = ids.length ? ids : [activeId];
    return resolvedIds
      .map((id) => sessions.find((session) => session.profile.id === id))
      .filter((session): session is SessionSummary => Boolean(session));
  }, [activeId, sessions, workspaceRoot]);

  const syncInputTargetCount = useMemo(() => resolveSyncInputTargets(
    activeId,
    paneSessions.map((session) => ({
      id: session.profile.id,
      kind: session.profile.kind,
      connected: session.runtime.status === "connected",
    })),
    syncInputSettings,
  ).length, [activeId, paneSessions, syncInputSettings]);

  function handleMenuAction(item: MenuItem | "会话搜索") {
    const workspacePanel = workspacePanelMenuItems[item];
    if (workspacePanel) {
      const visible = focusMode || !workspacePanels[workspacePanel];
      if (focusMode) setFocusMode(false);
      setWorkspacePanelVisible(workspacePanel, visible);
      return;
    }
    if (item === "终端设置") {
      setDialog("terminal");
      return;
    }
    if (item === "快速命令") {
      setUtilityDialog("quick-commands");
      return;
    }
    if (item === "OneKeys") {
      setUtilityDialog("one-keys");
      return;
    }
    if (item === "MCP Bridge") {
      setUtilityDialog("mcp");
      return;
    }
    if (item === "日志管理") {
      setUtilityDialog("logs");
      return;
    }
    if (item === "关于 PortMate") {
      setNotice({ title: "关于 PortMate", message: "PortMate 是面向串口、SSH 和 MCP 会话控制的桌面终端工作台。" });
      return;
    }
    if (item === "查找") {
      if (active) requestTerminalSearch();
      else setNotice({ title: "查找", message: "请先打开一个终端会话。" });
      return;
    }
    if (item === "自由输入") {
      if (active) requestTerminalFreeInput();
      else setNotice({ title: "自由输入", message: "请先打开一个终端会话。" });
      return;
    }
    if (item === "快捷栏") {
      if (focusMode) {
        setFocusMode(false);
        setQuickBarVisible(true);
      } else {
        setQuickBarVisible((visible) => !visible);
      }
      return;
    }
    if (item === "会话搜索") {
      setSearchDialog({ mode: "sessions", query: "" });
      setUtilityDialog("search");
      return;
    }
    if (item === "跳转到行") {
      if (active) requestTerminalGotoLine();
      else setNotice({ title: "跳转到行", message: "请先打开一个终端会话。" });
      return;
    }
    if (item === "导出终端文本" || item === "导出选中文本") {
      void exportTerminalText(item === "导出终端文本" ? "buffer" : "selection");
      return;
    }
    const terminalKeyMode = terminalKeyModeMenuItems[item];
    if (terminalKeyMode) {
      setActiveWorkspaceViewKeyMode(terminalKeyMode);
      return;
    }
    if (item === "同步输入") {
      updateSyncInput(!syncInputRef.current);
      return;
    }
    if (item === "块选择") {
      setBlockSelection((current) => !current);
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
      if (!active) return;
      setDraft(active.profile);
      setSessionSettingsSection("会话");
      setDialog("session");
      return;
    }
    if (["端口转发", "触发器", "密钥管理器"].includes(item)) {
      if (item === "端口转发") {
        if (!active || !isSshLikeProfile(active.profile) || active.runtime.status !== "connected") {
          setNotice({ title: "端口转发", message: "请选择一个已保存并已连接的 SSH/Tmux 会话后再创建 tunnel。" });
          return;
        }
        setUtilityDialog("tunnel");
        return;
      }
      if (item === "触发器") {
        if (!active) return;
        setDraft(active.profile);
        setSessionSettingsSection("触发器");
        setDialog("session");
        return;
      }
      setUtilityDialog("keys");
      return;
    }
    if (item === "Sysmon") {
      if (!active) {
        setNotice({ title: "Sysmon", message: "请先选择一个会话。" });
        return;
      }
      setUtilityDialog("sysmon");
      return;
    }
    if (item === "串口分析器") {
      if (!active || active.profile.connection.kind !== "serial") {
        setNotice({ title: "串口分析器", message: "请先选择一个串口会话。" });
        return;
      }
      void openSerialAnalyzer(active);
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
    if (item === "传输任务") {
      if (!active) {
        setNotice({ title: item, message: "请先选择一个会话。" });
        return;
      }
      setUtilityDialog("transfer");
      return;
    }
    if (item === "复制会话") {
      duplicateSessionFromContext();
      return;
    }
    if (item === "还原布局") {
      restoreWorkspaceLayout();
      return;
    }

    setNotice({ title: item, message: "未识别的菜单项。" });
  }

  function openAppContextMenu(event: ReactMouseEvent, sessionId?: string) {
    event.preventDefault();
    event.stopPropagation();
    const target = event.target instanceof Element ? event.target : null;
    const terminalHost = target?.closest<HTMLElement>(".terminal-host");
    const paneElement = terminalHost?.closest<HTMLElement>(".terminal-pane[data-pane-id]");
    const pane = paneElement?.dataset.paneId
      ? findWorkspacePane(workspaceRoot, paneElement.dataset.paneId)
      : undefined;
    const view = pane ? workspacePaneActiveView(pane) : undefined;
    const nextSessionId = sessionId ?? (activeId || sessions[0]?.profile.id || null);
    if (terminalHost && pane && view) {
      activateWorkspacePane(pane.id, view.id);
      setOpenMenu(null);
      setWorkspaceViewContextMenu(null);
      setContextMenu({
        kind: "terminal",
        x: event.clientX,
        y: event.clientY,
        paneId: pane.id,
        viewId: view.id,
        sessionId: view.sessionId,
        alternate: terminalHost.dataset.terminalBuffer === "alternate",
        hasSelection: terminalHost.dataset.terminalHasSelection === "true",
      });
      return;
    }
    if (sessionId) {
      activateSession(sessionId);
    }
    setOpenMenu(null);
    setWorkspaceViewContextMenu(null);
    setContextMenu({ kind: "session", x: event.clientX, y: event.clientY, sessionId: nextSessionId });
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

  async function saveSessionFromContext(sessionId?: string | null, activateWorkspace = true) {
    const session = contextSession(sessionId);
    if (!session) return;
    const saved = await saveProfile(prepareSessionProfile(session.profile));
    applySavedSession(saved, activateWorkspace);
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

  async function exportTerminalText(
    source: TerminalTextExportSource,
    target?: { sessionId: string; viewId: string },
  ) {
    const viewId = target?.viewId ?? activeWorkspaceView?.id;
    const sessionId = target?.sessionId ?? activeWorkspaceView?.sessionId;
    const session = sessions.find((candidate) => candidate.profile.id === sessionId);
    const title = source === "selection" ? "导出选中文本" : "导出终端文本";
    if (!session || !viewId) {
      setNotice({ title, message: "请先打开一个终端视图。" });
      return;
    }
    try {
      const payload = await requestTerminalTextExport({ sessionId: session.profile.id, viewId, source });
      if (payload.sessionId !== session.profile.id || payload.viewId !== viewId || payload.source !== source) {
        throw new Error("终端导出响应与目标视图不匹配。");
      }
      if (isBackendAvailable()) {
        const result = await invokeBackend<ExportTerminalTextResult>("export_terminal_text", {
          request: {
            sessionId: payload.sessionId,
            viewId: payload.viewId,
            source: payload.source,
            text: payload.text,
          },
        });
        setNotice({
          title,
          message: `${formatBytes(result.size)} · ${payload.logicalLines} 行 · SHA-256 ${result.sha256.slice(0, 16)}...\n${result.path}`,
        });
      } else {
        const { downloadTerminalText } = await import("./terminal-export-download");
        const fileName = downloadTerminalText(payload.text, session.profile.name, source);
        setNotice({ title, message: `已下载 ${fileName} · ${formatBytes(payload.bytes)} · ${payload.logicalLines} 行` });
      }
    } catch (error) {
      setNotice({ title, message: formatError(error) });
    }
  }

  async function runTerminalSelectionAction(
    action: TerminalSelectionAction,
    title: string,
    target?: { sessionId: string; viewId: string },
  ) {
    const sessionId = target?.sessionId ?? activeWorkspaceView?.sessionId;
    const viewId = target?.viewId ?? activeWorkspaceView?.id;
    if (!sessionId || !viewId) {
      setNotice({ title, message: "请先打开一个终端视图。" });
      return;
    }
    try {
      const { executeTerminalSelectionAction } = await import("./terminal-selection-event");
      await executeTerminalSelectionAction({ sessionId, viewId, action });
    } catch (error) {
      setNotice({ title, message: formatError(error) });
    }
  }

  async function searchTerminalOnline(target?: { sessionId: string; viewId: string }) {
    const sessionId = target?.sessionId ?? activeWorkspaceView?.sessionId;
    const viewId = target?.viewId ?? activeWorkspaceView?.id;
    const session = sessions.find((candidate) => candidate.profile.id === sessionId);
    if (!session || !viewId) {
      setNotice({ title: "在线搜索", message: "请先打开一个终端视图。" });
      return;
    }
    try {
      const { executeTerminalOnlineSearch } = await import("./terminal-selection-event");
      await executeTerminalOnlineSearch({
        sessionId: session.profile.id,
        viewId,
        fallback: session.lastLine,
      });
    } catch (error) {
      setNotice({ title: "在线搜索", message: formatError(error) });
    }
  }

  async function runTerminalBufferAction(
    action: TerminalBufferAction,
    title: string,
    target?: { sessionId: string; viewId: string },
  ) {
    const sessionId = target?.sessionId ?? activeWorkspaceView?.sessionId;
    const viewId = target?.viewId ?? activeWorkspaceView?.id;
    if (!sessionId || !viewId) {
      setNotice({ title, message: "请先打开一个终端视图。" });
      return;
    }
    try {
      const { executeTerminalBufferAction } = await import("./terminal-buffer-event");
      await executeTerminalBufferAction({ sessionId, viewId, action });
    } catch (error) {
      setNotice({ title, message: formatError(error) });
    }
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
    if (text) await routeTerminalInput(session.profile.id, text, "atomic");
  }

  async function closeSessionsByIds(ids: string[]) {
    const failed: string[] = [];
    for (const id of ids) {
      const disconnected = await disconnectSession(id, false, false);
      if (!disconnected) failed.push(id);
    }
    if (failed.length) {
      setNotice({ title: "断开会话", message: `${failed.length} 个会话断开失败，其余已断开。` });
    }
  }

  function applyDeletedSessionProfile(response: DeleteSessionProfileResponse) {
    const profileId = response.deletedProfileId;
    const remainingSessionIds = response.sessions.map((session) => session.profile.id);
    const reconciled = reconcileWorkspaceSnapshot({
      version: 4,
      root: workspaceRoot,
      activePaneId,
      activeId,
      tabColors,
    }, remainingSessionIds);

    setSessions(response.sessions);
    saveLocalSessionSummaries(response.sessions);
    setOneKeys(response.oneKeys);
    setHostKeys(response.hostKeys);
    setGrants(response.grants);
    setLogs((current) => Object.fromEntries(
      Object.entries(current).filter(([sessionId]) => sessionId !== profileId),
    ));
    setTransfers((current) => current.filter((transfer) => transfer.sessionId !== profileId));
    setSerialCaptures((current) => Object.fromEntries(
      Object.entries(current).filter(([sessionId]) => sessionId !== profileId),
    ));
    setMcpApprovals((current) => current.filter((approval) => approval.sessionId !== profileId));
    setClosedWorkspaceViews((current) => current.filter((closed) => closed.view.sessionId !== profileId));
    setTerminalPrefs((current) => ({
      ...current,
      startupSessions: current.startupSessions.filter((sessionId) => sessionId !== profileId),
    }));
    setWorkspaceRoot(reconciled.root);
    setActivePaneId(reconciled.activePaneId);
    setActiveId(reconciled.activeId);
    setTabColors(reconciled.tabColors);
    setZoomedPaneId((current) => current && findWorkspacePane(reconciled.root, current) ? current : "");
    setWorkspaceGroupMove(null);
    setWorkspaceViewRename(null);
    setWorkspaceViewContextMenu(null);
    setDraft((current) => current.id === profileId ? createSessionDraft() : current);
    setHostKeyPrompt((current) => current?.profile.id === profileId ? null : current);
    delete serialCapturesRef.current[profileId];
    delete serialCaptureEpochRef.current[profileId];
    serialCaptureRefreshesRef.current.delete(profileId);
    activeLogRefreshGateRef.current.invalidate(profileId);
    delete logSignatureRef.current[profileId];
    sessionsSignatureRef.current = "";
  }

  async function deleteSessionFromContext(sessionId?: string | null) {
    const target = contextSession(sessionId);
    if (!target) return;
    const confirmed = window.confirm(
      `删除会话 Profile “${target.profile.name}”？\n\n活动连接会先断开；内存历史、传输记录、Profile 级 Host Key 和会话绑定会删除。磁盘日志分片与安全审计保留。`,
    );
    if (!confirmed) return;

    try {
      const response = isBackendAvailable()
        ? await invokeBackend<DeleteSessionProfileResponse>("delete_session_profile", { sessionId: target.profile.id })
        : deleteSessionProfileFromClientState(target.profile.id, { sessions, oneKeys, hostKeys, grants });
      applyDeletedSessionProfile(response);
      setNotice({ title: "会话已删除", message: `已删除 ${target.profile.name}；磁盘日志仍可在日志管理器中查看或清理。` });
    } catch (error) {
      setNotice({ title: "删除会话失败", message: formatError(error) });
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

  function handleContextMenuAction(action: SessionContextAction, sessionId?: string | null) {
    setContextMenu(null);
    const target = contextSession(sessionId);
    switch (action) {
      case "sync-toggle":
        updateSyncInput(!syncInputRef.current);
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
        splitWorkspace(workspaceSplitDirectionForVisualOrientation("horizontal"));
        return;
      case "split-v":
        splitWorkspace(workspaceSplitDirectionForVisualOrientation("vertical"));
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
      case "delete-profile":
        void deleteSessionFromContext(sessionId);
        return;
      default:
        return;
    }
  }

  function handleTerminalContextMenuAction(
    action: TerminalContextAction,
    state: Extract<NonNullable<ContextMenuState>, { kind: "terminal" }>,
  ) {
    setContextMenu(null);
    activateWorkspacePane(state.paneId, state.viewId);
    const target = { sessionId: state.sessionId, viewId: state.viewId };
    switch (action) {
      case "copy":
        void runTerminalSelectionAction("copy", "复制", target);
        return;
      case "paste":
        void navigator.clipboard?.readText().then((text) => {
          if (text) return routeTerminalInput(state.sessionId, text, "atomic");
        }).catch((error) => setNotice({ title: "粘贴", message: formatError(error) }));
        return;
      case "find":
        window.requestAnimationFrame(() => requestTerminalSearch());
        return;
      case "search-online":
        void searchTerminalOnline(target);
        return;
      case "clear-scrollback":
        void runTerminalBufferAction(action, "清除回滚", target);
        return;
      case "clear-screen":
        void runTerminalBufferAction(action, "清除屏幕", target);
        return;
      case "clear-all":
        void runTerminalBufferAction(action, "清除屏幕和回滚", target);
        return;
      case "select-all":
        void runTerminalSelectionAction("select-all", "选择全部", target);
        return;
      case "clear-selection":
        void runTerminalSelectionAction("clear", "清除选择", target);
        return;
      case "export-buffer":
        void exportTerminalText("buffer", target);
        return;
      case "export-selection":
        void exportTerminalText("selection", target);
        return;
      case "triggers": {
        const session = sessions.find((candidate) => candidate.profile.id === state.sessionId);
        setDraft(session?.profile ?? createSessionDraft());
        setSessionSettingsSection("触发器");
        setDialog("session");
        return;
      }
    }
  }

  function openNewSessionDialog() {
    setDraft(createSessionDraft());
    setSessionSettingsSection("会话");
    setDialog("session");
  }

  function activateSession(sessionId: string) {
    const currentPane = findWorkspacePane(workspaceRoot, activePaneId);
    const existingPane = currentPane?.sessionIds.includes(sessionId)
      ? currentPane
      : findWorkspacePaneBySession(workspaceRoot, sessionId);
    if (existingPane) {
      setWorkspaceRoot((current) => activateWorkspacePaneSession(current, existingPane.id, sessionId));
      setActivePaneId(existingPane.id);
      setActiveId(sessionId);
      setZoomedPaneId((current) => current ? existingPane.id : "");
      return;
    }
    if (!workspaceRoot) {
      const pane = createWorkspacePane(sessionId);
      setWorkspaceRoot(pane);
      setActivePaneId(pane.id);
    } else {
      const targetPane = currentPane ?? workspacePaneLeaves(workspaceRoot)[0];
      if (targetPane) {
        const nextRoot = addWorkspacePaneSession(workspaceRoot, targetPane.id, sessionId);
        if (nextRoot === workspaceRoot && !targetPane.sessionIds.includes(sessionId)) {
          setNotice({ title: "打开视图失败", message: `每个分组最多包含 ${MAX_WORKSPACE_GROUP_TABS} 个视图。` });
          return;
        }
        setWorkspaceRoot(nextRoot);
        setActivePaneId(targetPane.id);
      }
    }
    setActiveId(sessionId);
  }

  function activateWorkspacePane(paneId: string, viewId: string) {
    const pane = findWorkspacePane(workspaceRoot, paneId);
    const view = pane?.views.find((candidate) => candidate.id === viewId);
    if (!pane || !view) return;
    setWorkspaceRoot((current) => activateWorkspacePaneView(current, paneId, viewId));
    setActivePaneId(paneId);
    setActiveId(view.sessionId);
    setZoomedPaneId((current) => current ? paneId : "");
  }

  function cycleActiveWorkspaceView(offset: -1 | 1) {
    const pane = findWorkspacePane(workspaceRoot, activePaneId);
    const view = pane ? workspacePaneViewAtOffset(pane, offset) : undefined;
    if (!pane || !view) return;
    if (view.id !== pane.activeViewId) activateWorkspacePane(pane.id, view.id);
    focusWorkspacePaneInput(pane.id);
  }

  function setActiveWorkspaceViewKeyMode(
    keyMode: TerminalKeyMode,
    paneId = activePaneId,
    viewId = findWorkspacePane(workspaceRoot, paneId)?.activeViewId,
  ) {
    if (!viewId) return;
    setWorkspaceRoot((current) => setWorkspacePaneViewKeyMode(current, paneId, viewId, keyMode));
    focusWorkspacePaneInput(paneId);
  }

  function restoreWorkspaceLayout() {
    const restored = reconcileWorkspaceSnapshot(
      loadWorkspaceSnapshot(),
      sessions.map((session) => session.profile.id),
    );
    setWorkspaceRoot(restored.root);
    setActivePaneId(restored.activePaneId);
    setActiveId(restored.activeId);
    setTabColors(restored.tabColors);
    setZoomedPaneId("");
    setNotice({
      title: "还原布局",
      message: workspacePaneLeaves(restored.root).length <= 1
        ? "已还原单窗格工作区。"
        : `已还原 ${workspacePaneLeaves(restored.root).length} 个窗格。`,
    });
  }

  function splitWorkspace(
    direction: WorkspaceSplitDirection,
    placement: WorkspaceSplitPlacement = "second",
    paneId = activePaneId,
    sessionId = activeId,
  ) {
    const primaryId = sessionId || sessions[0]?.profile.id;
    if (!primaryId) {
      openNewSessionDialog();
      return;
    }
    const root = workspaceRoot ?? createWorkspacePane(primaryId, paneId || createWorkspaceNodeId("pane"));
    const panes = workspacePaneLeaves(root);
    if (panes.length >= MAX_WORKSPACE_PANES) {
      setNotice({ title: "分屏", message: `最多同时打开 ${MAX_WORKSPACE_PANES} 个窗格。` });
      return;
    }
    const targetPane = findWorkspacePane(root, paneId)
      ?? findWorkspacePaneBySession(root, primaryId)
      ?? panes[0];
    if (!targetPane) return;
    const openSessionIds = new Set(panes.flatMap((pane) => pane.views.map((view) => view.sessionId)));
    const nextId = sessions.find((session) => !openSessionIds.has(session.profile.id))?.profile.id ?? primaryId;
    const nextRoot = splitWorkspacePane(
      root,
      targetPane.id,
      direction,
      nextId,
      createWorkspaceNodeId("pane"),
      createWorkspaceNodeId("split"),
      placement,
    );
    if (nextRoot === root) {
      setNotice({ title: "分屏", message: `嵌套分屏最多支持 ${MAX_WORKSPACE_DEPTH} 层。` });
      return;
    }
    setWorkspaceRoot(nextRoot);
    setActivePaneId(targetPane.id);
    setActiveId(workspacePaneActiveView(targetPane).sessionId);
    setZoomedPaneId("");
  }

  function pushClosedWorkspaceViews(views: ClosedWorkspaceView[]) {
    if (!views.length) return;
    setClosedWorkspaceViews((current) => (
      [...current, ...views].slice(-MAX_CLOSED_WORKSPACE_VIEWS)
    ));
  }

  function duplicateActiveWorkspaceView(paneId = activePaneId, viewId?: string) {
    const pane = findWorkspacePane(workspaceRoot, paneId);
    const source = pane?.views.find((view) => view.id === (viewId ?? pane.activeViewId));
    if (!pane || !source) return;
    if (pane.views.length >= MAX_WORKSPACE_GROUP_TABS) {
      setNotice({ title: "复制视图", message: `每个分组最多包含 ${MAX_WORKSPACE_GROUP_TABS} 个视图。` });
      return;
    }
    const sessionName = sessions.find((session) => session.profile.id === source.sessionId)?.profile.name ?? "会话";
    const baseTitle = source.title || sessionName;
    const labels = new Set(pane.views.map((view) => (
      view.title || sessions.find((session) => session.profile.id === view.sessionId)?.profile.name || "会话"
    )));
    let duplicateTitle = `${baseTitle} 副本`;
    let suffix = 2;
    while (labels.has(duplicateTitle)) {
      duplicateTitle = `${baseTitle} 副本 ${suffix}`;
      suffix += 1;
    }
    const duplicateId = createWorkspaceNodeId("view");
    const nextRoot = duplicateWorkspacePaneView(workspaceRoot, pane.id, source.id, duplicateId, duplicateTitle);
    if (nextRoot === workspaceRoot) return;
    setWorkspaceRoot(nextRoot);
    setActivePaneId(pane.id);
    setActiveId(source.sessionId);
    focusWorkspacePaneInput(pane.id);
  }

  function openWorkspaceViewRename(paneId = activePaneId, viewId?: string) {
    const pane = findWorkspacePane(workspaceRoot, paneId);
    const view = pane?.views.find((candidate) => candidate.id === (viewId ?? pane.activeViewId));
    const sessionName = sessions.find((session) => session.profile.id === view?.sessionId)?.profile.name;
    if (!pane || !view || !sessionName) return;
    setWorkspaceViewRename({
      paneId: pane.id,
      viewId: view.id,
      value: view.title || sessionName,
      sessionName,
    });
  }

  function commitWorkspaceViewRename(useSessionName = false) {
    if (!workspaceViewRename) return;
    const title = useSessionName ? "" : workspaceViewRename.value.trim();
    if (!useSessionName && !title) return;
    const storedTitle = title === workspaceViewRename.sessionName ? "" : title;
    setWorkspaceRoot((current) => renameWorkspacePaneView(
      current,
      workspaceViewRename.paneId,
      workspaceViewRename.viewId,
      storedTitle,
    ));
    setWorkspaceViewRename(null);
    focusWorkspacePaneInput(workspaceViewRename.paneId);
  }

  function closeWorkspaceViewRename() {
    const paneId = workspaceViewRename?.paneId;
    setWorkspaceViewRename(null);
    if (paneId) focusWorkspacePaneInput(paneId);
  }

  function openWorkspaceViewContextMenu(paneId: string, viewId: string, x: number, y: number) {
    const pane = findWorkspacePane(workspaceRoot, paneId);
    const view = pane?.views.find((candidate) => candidate.id === viewId);
    if (!pane || !view) return;
    activateWorkspacePane(pane.id, view.id);
    setOpenMenu(null);
    setContextMenu(null);
    setWorkspaceViewContextMenu({ x, y, paneId: pane.id, viewId: view.id });
  }

  function changeWorkspaceViewColor(paneId: string, viewId: string, color: string) {
    setWorkspaceRoot((current) => setWorkspacePaneViewColor(current, paneId, viewId, color));
    setWorkspaceViewContextMenu(null);
    focusWorkspacePaneInput(paneId);
  }

  function closeWorkspaceViews(paneId: string, viewIds: string[]) {
    const panes = workspacePaneLeaves(workspaceRoot);
    const source = panes.find((pane) => pane.id === paneId);
    if (!source) return;
    const requested = new Set(viewIds);
    const closedViews = source.views
      .map((view, index) => ({ view, paneId, index }))
      .filter((item) => requested.has(item.view.id));
    if (!closedViews.length) return;
    const totalViewCount = panes.reduce((count, pane) => count + pane.views.length, 0);
    if (closedViews.length >= totalViewCount) {
      setNotice({ title: "关闭视图", message: "工作区中至少需要保留一个视图。" });
      return;
    }

    let nextRoot = workspaceRoot;
    for (const item of closedViews) {
      nextRoot = removeWorkspacePaneView(nextRoot, paneId, item.view.id);
    }
    if (nextRoot === workspaceRoot) return;
    const nextPanes = workspacePaneLeaves(nextRoot);
    const sourceIndex = panes.findIndex((pane) => pane.id === paneId);
    const nextActive = nextPanes.find((pane) => pane.id === activePaneId)
      ?? nextPanes[Math.min(Math.max(0, sourceIndex), nextPanes.length - 1)];
    if (!nextActive) return;
    const shouldRefocus = nextActive.id !== activePaneId
      || (paneId === activePaneId && closedViews.some((item) => item.view.id === source.activeViewId));
    setWorkspaceRoot(nextRoot);
    setActivePaneId(nextActive.id);
    setActiveId(workspacePaneActiveView(nextActive).sessionId);
    setZoomedPaneId((current) => current && !findWorkspacePane(nextRoot, current) ? nextActive.id : current);
    pushClosedWorkspaceViews(closedViews);
    if (shouldRefocus) focusWorkspacePaneInput(nextActive.id);
  }

  function closeActiveWorkspaceView() {
    const pane = findWorkspacePane(workspaceRoot, activePaneId);
    if (pane) closeWorkspaceViews(pane.id, [pane.activeViewId]);
  }

  function closeOtherWorkspaceViews(paneId = activePaneId, viewId?: string) {
    const pane = findWorkspacePane(workspaceRoot, paneId);
    if (!pane) return;
    const activeViewId = viewId ?? pane.activeViewId;
    const viewIds = pane.views.filter((view) => view.id !== activeViewId).map((view) => view.id);
    if (!viewIds.length) {
      setNotice({ title: "关闭其他视图", message: "当前分组没有其他视图。" });
      return;
    }
    closeWorkspaceViews(pane.id, viewIds);
  }

  function closeRightWorkspaceViews(paneId = activePaneId, viewId?: string) {
    const pane = findWorkspacePane(workspaceRoot, paneId);
    if (!pane) return;
    const activeIndex = pane.views.findIndex((view) => view.id === (viewId ?? pane.activeViewId));
    if (activeIndex < 0) return;
    const viewIds = pane.views.slice(activeIndex + 1).map((view) => view.id);
    if (!viewIds.length) {
      setNotice({ title: "关闭右侧视图", message: "活动视图右侧没有其他视图。" });
      return;
    }
    closeWorkspaceViews(pane.id, viewIds);
  }

  function reopenClosedWorkspaceView() {
    let historyIndex = closedWorkspaceViews.length - 1;
    while (historyIndex >= 0 && !sessions.some((session) => session.profile.id === closedWorkspaceViews[historyIndex].view.sessionId)) {
      historyIndex -= 1;
    }
    if (historyIndex < 0) {
      setClosedWorkspaceViews([]);
      setNotice({ title: "重新打开已关闭视图", message: "没有可重新打开的视图。" });
      return;
    }
    const closedView = closedWorkspaceViews[historyIndex];
    const panes = workspacePaneLeaves(workspaceRoot);
    const candidateIds = [closedView.paneId, activePaneId, ...panes.map((pane) => pane.id)];
    const target = candidateIds
      .filter((paneId, index) => paneId && candidateIds.indexOf(paneId) === index)
      .map((paneId) => panes.find((pane) => pane.id === paneId))
      .find((pane) => pane && (
        pane.views.length < MAX_WORKSPACE_GROUP_TABS
      ));
    if (!target || !workspaceRoot) {
      setNotice({ title: "重新打开已关闭视图", message: `所有分组均已达到 ${MAX_WORKSPACE_GROUP_TABS} 个视图。` });
      return;
    }
    const insertionIndex = target.id === closedView.paneId
      ? closedView.index
      : target.views.length;
    const nextRoot = insertWorkspacePaneView(
      workspaceRoot,
      target.id,
      closedView.view,
      insertionIndex,
    );
    if (nextRoot === workspaceRoot) return;
    setWorkspaceRoot(nextRoot);
    setActivePaneId(target.id);
    setActiveId(closedView.view.sessionId);
    setZoomedPaneId((current) => current ? target.id : "");
    setClosedWorkspaceViews((current) => current.slice(0, historyIndex));
    focusWorkspacePaneInput(target.id);
  }

  function handleWorkspaceViewContextAction(
    action: WorkspaceViewContextAction,
    paneId: string,
    viewId: string,
  ) {
    const pane = findWorkspacePane(workspaceRoot, paneId);
    const view = pane?.views.find((candidate) => candidate.id === viewId);
    if (!pane || !view) return;
    setWorkspaceViewContextMenu(null);
    switch (action) {
      case "copy-name":
        copySessionNameFromContext(view.sessionId);
        return;
      case "copy-url":
        copySessionUrlFromContext(view.sessionId);
        return;
      case "reconnect":
        void connectSession(view.sessionId, undefined, false);
        return;
      case "save":
        void saveSessionFromContext(view.sessionId, false);
        return;
      case "export-buffer":
        void exportTerminalText("buffer", { sessionId: view.sessionId, viewId: view.id });
        return;
      case "export-selection":
        void exportTerminalText("selection", { sessionId: view.sessionId, viewId: view.id });
        return;
      case "split-horizontal":
        splitWorkspace(workspaceSplitDirectionForVisualOrientation("horizontal"), "second", pane.id, view.sessionId);
        return;
      case "split-vertical":
        splitWorkspace(workspaceSplitDirectionForVisualOrientation("vertical"), "second", pane.id, view.sessionId);
        return;
      case "move-group":
        setWorkspaceGroupMove({ paneId: pane.id, mode: "view" });
        return;
      case "move-new-left":
        splitWorkspaceViewToGroup("vertical", "first", pane.id, view.id);
        return;
      case "move-new-right":
        splitWorkspaceViewToGroup("vertical", "second", pane.id, view.id);
        return;
      case "move-new-up":
        splitWorkspaceViewToGroup("horizontal", "first", pane.id, view.id);
        return;
      case "move-new-down":
        splitWorkspaceViewToGroup("horizontal", "second", pane.id, view.id);
        return;
      case "detach-pane":
        void detachWorkspacePane(pane.id);
        return;
      case "merge-group":
        setWorkspaceGroupMove({ paneId: pane.id, mode: "group" });
        return;
      case "swap-up":
        swapWorkspacePane("up", pane.id);
        return;
      case "swap-down":
        swapWorkspacePane("down", pane.id);
        return;
      case "swap-left":
        swapWorkspacePane("left", pane.id);
        return;
      case "swap-right":
        swapWorkspacePane("right", pane.id);
        return;
      case "toggle-zoom":
        toggleWorkspaceZoom(pane.id);
        return;
      case "close":
        closeWorkspaceViews(pane.id, [view.id]);
        return;
      case "close-other":
        closeOtherWorkspaceViews(pane.id, view.id);
        return;
      case "close-right":
        closeRightWorkspaceViews(pane.id, view.id);
        return;
      case "reopen":
        reopenClosedWorkspaceView();
        return;
      case "close-pane":
        closeWorkspacePane(pane.id);
        return;
      case "settings":
        openSessionSettingsFromContext(view.sessionId);
        return;
    }
  }

  function closeWorkspacePane(paneId = activePaneId, recordHistory = true) {
    const panes = workspacePaneLeaves(workspaceRoot);
    if (panes.length <= 1 || !paneId) return;
    const removedIndex = panes.findIndex((pane) => pane.id === paneId);
    if (removedIndex < 0) return;
    const removedPane = panes[removedIndex];
    const nextRoot = removeWorkspacePane(workspaceRoot, paneId);
    const nextPanes = workspacePaneLeaves(nextRoot);
    const currentActive = nextPanes.find((pane) => pane.id === activePaneId);
    const nextActive = currentActive ?? nextPanes[Math.min(removedIndex, nextPanes.length - 1)];
    setWorkspaceRoot(nextRoot);
    setActivePaneId(nextActive?.id ?? "");
    setActiveId(nextActive ? workspacePaneActiveView(nextActive).sessionId : activeId);
    setZoomedPaneId((current) => current ? nextActive?.id ?? "" : "");
    if (recordHistory) {
      pushClosedWorkspaceViews(removedPane.views.map((view, index) => ({ view, paneId, index })));
    }
  }

  function splitWorkspaceViewToGroup(
    direction: WorkspaceSplitDirection,
    placement: WorkspaceSplitPlacement,
    sourcePaneId = activePaneId,
    sourceViewId?: string,
  ) {
    const panes = workspacePaneLeaves(workspaceRoot);
    const source = findWorkspacePane(workspaceRoot, sourcePaneId);
    if (!workspaceRoot || !source) return;
    if (source.views.length <= 1) {
      setNotice({ title: "视图拆分到新分组", message: "当前分组至少需要保留一个其他视图。" });
      return;
    }
    if (panes.length >= MAX_WORKSPACE_PANES) {
      setNotice({ title: "视图拆分到新分组", message: `工作区最多支持 ${MAX_WORKSPACE_PANES} 个分组。` });
      return;
    }
    const newPaneId = createWorkspaceNodeId("pane");
    const activeView = source.views.find((view) => view.id === sourceViewId) ?? workspacePaneActiveView(source);
    const nextRoot = splitWorkspacePaneViewToGroup(
      workspaceRoot,
      source.id,
      activeView.id,
      direction,
      newPaneId,
      createWorkspaceNodeId("split"),
      placement,
    );
    if (nextRoot === workspaceRoot) {
      setNotice({ title: "视图拆分到新分组", message: `嵌套分组最多支持 ${MAX_WORKSPACE_DEPTH} 层。` });
      return;
    }
    setWorkspaceRoot(nextRoot);
    setActivePaneId(newPaneId);
    setActiveId(activeView.sessionId);
    setZoomedPaneId("");
    focusWorkspacePaneInput(newPaneId);
  }

  function splitWorkspaceViewFromDrop(
    sourcePaneId: string,
    viewId: string,
    targetPaneId: string,
    edge: WorkspacePaneDirection,
  ) {
    const source = findWorkspacePane(workspaceRoot, sourcePaneId);
    const target = findWorkspacePane(workspaceRoot, targetPaneId);
    const view = source?.views.find((candidate) => candidate.id === viewId);
    if (!workspaceRoot || !source || !target || !view) return;
    if (sourcePaneId === targetPaneId && source.views.length <= 1) {
      setNotice({ title: "拖放视图", message: "最终视图不能拆成空分组。" });
      return;
    }
    const paneDelta = source.views.length > 1 ? 1 : 0;
    if (workspacePaneLeaves(workspaceRoot).length + paneDelta > MAX_WORKSPACE_PANES) {
      setNotice({ title: "拖放视图", message: `工作区最多支持 ${MAX_WORKSPACE_PANES} 个分组。` });
      return;
    }
    const direction: WorkspaceSplitDirection = edge === "left" || edge === "right" ? "vertical" : "horizontal";
    const placement: WorkspaceSplitPlacement = edge === "left" || edge === "up" ? "first" : "second";
    const newPaneId = createWorkspaceNodeId("pane");
    const nextRoot = moveWorkspacePaneViewToNewGroup(
      workspaceRoot,
      sourcePaneId,
      targetPaneId,
      viewId,
      direction,
      newPaneId,
      createWorkspaceNodeId("split"),
      placement,
    );
    if (nextRoot === workspaceRoot) {
      setNotice({ title: "拖放视图", message: `嵌套分组最多支持 ${MAX_WORKSPACE_DEPTH} 层。` });
      return;
    }
    setWorkspaceRoot(nextRoot);
    setActivePaneId(newPaneId);
    setActiveId(view.sessionId);
    setZoomedPaneId("");
    focusWorkspacePaneInput(newPaneId);
  }

  function moveWorkspaceView(sourcePaneId: string, targetPaneId: string) {
    const source = findWorkspacePane(workspaceRoot, sourcePaneId);
    if (!source || sourcePaneId === targetPaneId) return;
    const activeView = workspacePaneActiveView(source);
    moveWorkspaceViewToIndex(sourcePaneId, activeView.id, targetPaneId, Number.POSITIVE_INFINITY);
  }

  function moveWorkspaceViewToIndex(
    sourcePaneId: string,
    viewId: string,
    targetPaneId: string,
    targetIndex: number,
  ) {
    const source = findWorkspacePane(workspaceRoot, sourcePaneId);
    const target = findWorkspacePane(workspaceRoot, targetPaneId);
    const view = source?.views.find((candidate) => candidate.id === viewId);
    if (!source || !target || !view) return;
    if (sourcePaneId !== targetPaneId && target.views.length >= MAX_WORKSPACE_GROUP_TABS) {
      setNotice({ title: "移动视图到分组", message: `每个分组最多包含 ${MAX_WORKSPACE_GROUP_TABS} 个视图。` });
      return;
    }
    const nextRoot = moveWorkspacePaneView(workspaceRoot, sourcePaneId, targetPaneId, view.id, targetIndex);
    if (nextRoot === workspaceRoot) return;
    setWorkspaceRoot(nextRoot);
    setActivePaneId(targetPaneId);
    setActiveId(view.sessionId);
    setZoomedPaneId((current) => current ? targetPaneId : "");
    setWorkspaceGroupMove(null);
    focusWorkspacePaneInput(targetPaneId);
  }

  function mergeWorkspaceGroup(sourcePaneId: string, targetPaneId: string) {
    const source = findWorkspacePane(workspaceRoot, sourcePaneId);
    const target = findWorkspacePane(workspaceRoot, targetPaneId);
    if (!source || !target || sourcePaneId === targetPaneId) return;
    if (target.views.length + source.views.length > MAX_WORKSPACE_GROUP_TABS) {
      setNotice({ title: "合并当前分组", message: `每个分组最多包含 ${MAX_WORKSPACE_GROUP_TABS} 个视图。` });
      return;
    }
    const nextRoot = mergeWorkspacePaneGroups(workspaceRoot, sourcePaneId, targetPaneId);
    if (nextRoot === workspaceRoot) return;
    setWorkspaceRoot(nextRoot);
    setActivePaneId(targetPaneId);
    setActiveId(workspacePaneActiveView(source).sessionId);
    setZoomedPaneId("");
    setWorkspaceGroupMove(null);
    focusWorkspacePaneInput(targetPaneId);
  }

  async function detachWorkspacePane(paneId = activePaneId) {
    const panes = workspacePaneLeaves(workspaceRoot);
    const pane = panes.find((item) => item.id === paneId);
    const activeView = pane ? workspacePaneActiveView(pane) : undefined;
    const session = activeView ? sessions.find((item) => item.profile.id === activeView.sessionId) : undefined;
    if (!pane || !session) return;
    if (panes.length <= 1 && pane.views.length <= 1) {
      setNotice({ title: "移到新窗口", message: "主窗口中至少需要保留一个窗格或视图。" });
      return;
    }
    const request: DetachedPaneRequest = {
      windowId: createWorkspaceNodeId("pane").replace(/[^A-Za-z0-9_-]/g, "-"),
      paneId: pane.id,
      viewId: activeView!.id,
      sessionId: activeView!.sessionId,
      title: activeView!.title,
      color: activeView!.color,
      keyMode: activeView!.keyMode,
    };
    try {
      await openDetachedPaneWindow(request, activeView!.title || session.profile.name);
      if (pane.views.length > 1) {
        const nextRoot = removeWorkspacePaneView(workspaceRoot, pane.id, activeView!.id);
        const nextPane = findWorkspacePane(nextRoot, pane.id);
        setWorkspaceRoot(nextRoot);
        setActivePaneId(nextPane?.id ?? "");
        setActiveId(nextPane ? workspacePaneActiveView(nextPane).sessionId : activeId);
        setZoomedPaneId((current) => current ? nextPane?.id ?? "" : "");
      } else {
        closeWorkspacePane(pane.id, false);
      }
    } catch (error) {
      setNotice({ title: "移到新窗口失败", message: formatError(error) });
    }
  }

  async function openSerialAnalyzer(session: SessionSummary) {
    const request: SerialAnalyzerRequest = {
      windowId: createWorkspaceNodeId("pane").replace(/^pane-/, "serial-analyzer-").replace(/[^A-Za-z0-9_-]/g, "-"),
      sessionId: session.profile.id,
    };
    try {
      const { openSerialAnalyzerWindow } = await import("./serial-analyzer-window");
      await openSerialAnalyzerWindow(request, session.profile.name);
    } catch (error) {
      setNotice({ title: "打开串口分析器失败", message: formatError(error) });
    }
  }

  function reattachDetachedPane(command: DetachedPaneCommand) {
    const session = sessions.find((item) => item.profile.id === command.sessionId);
    if (!session) {
      setNotice({ title: "返回主窗口失败", message: "原会话已不存在。" });
      return;
    }
    const returnedView: WorkspaceView = { id: command.viewId, sessionId: command.sessionId, title: command.title, color: command.color, keyMode: command.keyMode };
    const alreadyReturned = workspacePaneLeaves(workspaceRoot).find((pane) => pane.views.some((view) => view.id === command.viewId));
    if (alreadyReturned) {
      setWorkspaceRoot(activateWorkspacePaneView(workspaceRoot, alreadyReturned.id, command.viewId));
      setActivePaneId(alreadyReturned.id);
      setActiveId(command.sessionId);
      focusWorkspacePaneInput(alreadyReturned.id);
      return;
    }
    const existingPane = findWorkspacePane(workspaceRoot, command.paneId);
    if (existingPane) {
      const replacedFullGroup = existingPane.views.length >= MAX_WORKSPACE_GROUP_TABS;
      if (replacedFullGroup) {
        const replacedView = workspacePaneActiveView(existingPane);
        pushClosedWorkspaceViews([{ view: replacedView, paneId: existingPane.id, index: existingPane.views.findIndex((view) => view.id === replacedView.id) }]);
      }
      setWorkspaceRoot(replacedFullGroup
        ? replaceWorkspacePaneView(workspaceRoot, existingPane.id, returnedView)
        : insertWorkspacePaneView(workspaceRoot, existingPane.id, returnedView, existingPane.views.length));
      setActivePaneId(existingPane.id);
      setActiveId(command.sessionId);
      focusWorkspacePaneInput(existingPane.id);
      if (replacedFullGroup) {
        setNotice({ title: "窗格已返回", message: `原分组已达到 ${MAX_WORKSPACE_GROUP_TABS} 个视图，已替换该分组的活动视图。` });
      }
      return;
    }
    const panes = workspacePaneLeaves(workspaceRoot);
    if (!workspaceRoot) {
      const pane = createWorkspacePaneFromViews(command.paneId, [returnedView], returnedView.id)!;
      setWorkspaceRoot(pane);
      setActivePaneId(pane.id);
      setActiveId(returnedView.sessionId);
      focusWorkspacePaneInput(pane.id);
      return;
    }
    const target = findWorkspacePane(workspaceRoot, activePaneId) ?? panes[0];
    if (!target) return;
    if (panes.length >= MAX_WORKSPACE_PANES) {
      const replacedFullGroup = target.views.length >= MAX_WORKSPACE_GROUP_TABS;
      if (replacedFullGroup) {
        const replacedView = workspacePaneActiveView(target);
        pushClosedWorkspaceViews([{ view: replacedView, paneId: target.id, index: target.views.findIndex((view) => view.id === replacedView.id) }]);
      }
      setWorkspaceRoot(replacedFullGroup
        ? replaceWorkspacePaneView(workspaceRoot, target.id, returnedView)
        : insertWorkspacePaneView(workspaceRoot, target.id, returnedView, target.views.length));
      setActivePaneId(target.id);
      setActiveId(command.sessionId);
      setZoomedPaneId("");
      focusWorkspacePaneInput(target.id);
      setNotice({ title: "窗格已返回", message: `工作区已达到 ${MAX_WORKSPACE_PANES} 个窗格，已在当前窗格打开返回的会话。` });
      return;
    }
    let nextRoot = workspaceRoot;
    for (const candidate of [target, ...panes.filter((pane) => pane.id !== target.id)]) {
      const candidateRoot = splitWorkspacePaneWithView(
        workspaceRoot,
        candidate.id,
        "vertical",
        returnedView,
        command.paneId,
        createWorkspaceNodeId("split"),
        "second",
      );
      if (candidateRoot !== workspaceRoot) {
        nextRoot = candidateRoot;
        break;
      }
    }
    if (nextRoot === workspaceRoot) {
      const replacedFullGroup = target.views.length >= MAX_WORKSPACE_GROUP_TABS;
      if (replacedFullGroup) {
        const replacedView = workspacePaneActiveView(target);
        pushClosedWorkspaceViews([{ view: replacedView, paneId: target.id, index: target.views.findIndex((view) => view.id === replacedView.id) }]);
      }
      setWorkspaceRoot(replacedFullGroup
        ? replaceWorkspacePaneView(workspaceRoot, target.id, returnedView)
        : insertWorkspacePaneView(workspaceRoot, target.id, returnedView, target.views.length));
      setActivePaneId(target.id);
      setActiveId(command.sessionId);
      setZoomedPaneId("");
      focusWorkspacePaneInput(target.id);
      setNotice({ title: "窗格已返回", message: `所有窗格均已达到 ${MAX_WORKSPACE_DEPTH} 层深度，已在当前窗格打开返回的会话。` });
      return;
    }
    setWorkspaceRoot(nextRoot);
    setActivePaneId(command.paneId);
    setActiveId(command.sessionId);
    setZoomedPaneId("");
    focusWorkspacePaneInput(command.paneId);
  }

  function swapWorkspacePane(direction: WorkspacePaneDirection, sourcePaneId = activePaneId) {
    const nextPane = findWorkspacePaneInDirection(workspaceRoot, sourcePaneId, direction);
    if (!nextPane) {
      setNotice({ title: "交换窗格", message: "该方向没有可交换的窗格。" });
      return;
    }
    setWorkspaceRoot(swapWorkspacePanes(workspaceRoot, sourcePaneId, nextPane.id));
    focusWorkspacePaneInput(sourcePaneId);
  }

  function toggleWorkspaceZoom(paneId = activePaneId) {
    if (workspacePaneLeaves(workspaceRoot).length <= 1 || !paneId) return;
    setZoomedPaneId((current) => current === paneId ? "" : paneId);
    focusWorkspacePaneInput(paneId);
  }

  async function saveDraft(proxyPasswordUpdate: ProxyPasswordUpdate = null) {
    const profile = prepareSessionProfile(draft);
    try {
      const saved = await saveProfile(profile, proxyPasswordUpdate);
      applySavedSession(saved);
      setDraft(saved.profile);
      setDialog(null);
    } catch (error) {
      setNotice({ title: "保存会话失败", message: formatError(error) });
    }
  }

  async function saveDraftAndConnect(proxyPasswordUpdate: ProxyPasswordUpdate = null) {
    const profile = prepareSessionProfile(draft);
    try {
      const saved = await saveProfile(profile, proxyPasswordUpdate);
      applySavedSession(saved);
      setDraft(saved.profile);
      setDialog(null);
      await connectSession(saved.profile.id, saved);
    } catch (error) {
      setNotice({ title: "保存会话失败", message: formatError(error) });
    }
  }

  async function saveProfile(profile: SessionProfile, proxyPasswordUpdate: ProxyPasswordUpdate = null) {
    if (isBackendAvailable()) {
      const expectedProfile = sessions.find((session) => session.profile.id === profile.id)?.profile ?? null;
      return invokeBackend<SessionSummary>("save_session_profile", { profile, expectedProfile, proxyPasswordUpdate });
    }
    return createSessionSummary(profile);
  }

  function applySavedSession(saved: SessionSummary, activateWorkspace = true) {
    if (activateWorkspace) activateSession(saved.profile.id);
    setSessions((current) => {
      const nextSessions = mergeSessionSummaries(current, saved);
      saveLocalSessionSummaries(nextSessions);
      return nextSessions;
    });
  }

  function applySavedSessions(saved: SessionSummary[]) {
    if (!saved.length) return;
    setSessions((current) => {
      const nextSessions = saved.reduce(mergeSessionSummaries, current);
      saveLocalSessionSummaries(nextSessions);
      return nextSessions;
    });
  }

  async function connectSession(sessionId = activeId, sessionOverride?: SessionSummary, activateWorkspace = true) {
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
    if (activateWorkspace) activateSession(profileForConnect.id);

    try {
      const persisted = await saveProfile(profileForConnect);
      applySavedSession(persisted, activateWorkspace);
      const saved = isBackendAvailable()
        ? credentials.oneKeyId
          ? await invokeBackend<SessionSummary>("open_session_with_one_key", { sessionId: persisted.profile.id, oneKeyId: credentials.oneKeyId })
          : await invokeBackend<SessionSummary>("open_session", { sessionId: persisted.profile.id, password: credentials.password, passphrase: credentials.passphrase })
        : setSessionStatus(persisted, "connected");
      const fallbackLog = [...(logs[persisted.profile.id] ?? []), createLocalSystemEvent(saved.profile, `PortMate: connected to ${describeProfileEndpoint(saved.profile)}`)];
      const nextLog = await callBackend("tail_log", { sessionId: persisted.profile.id, limit: 600 }, fallbackLog);

      replaceSessionLog(persisted.profile.id, nextLog);
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
      replaceSessionLog(profileForConnect.id, nextLog);
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

  async function disconnectSession(sessionId = activeId, activateWorkspace = true, reportError = true): Promise<boolean> {
    const session = sessions.find((item) => item.profile.id === sessionId);
    if (!session) return false;
    if (isBackendAvailable() && session.runtime.status === "disconnected") return true;

    try {
      const saved = isBackendAvailable()
        ? await invokeBackend<SessionSummary>("close_session", { sessionId })
        : setSessionStatus(session, "disconnected");
      const fallbackLog = [...(logs[sessionId] ?? []), createLocalSystemEvent(saved.profile, "PortMate: session disconnected")];
      const nextLog = await callBackend("tail_log", { sessionId, limit: 160 }, fallbackLog);

      replaceSessionLog(sessionId, nextLog);
      if (activateWorkspace) activateSession(sessionId);
      setSessions((current) => {
        const nextSessions = mergeSessionSummaries(current, saved);
        saveLocalSessionSummaries(nextSessions);
        return nextSessions;
      });
      return true;
    } catch (error) {
      if (reportError) setNotice({ title: "断开会话失败", message: formatError(error) });
      return false;
    }
  }

  function routeTerminalInput(sessionId: string, text: string, origin: SyncInputOrigin = "interactive"): Promise<void> {
    const broadcastEnabled = syncInputRef.current;
    const settings = syncInputSettings;
    const candidates = paneSessions.map((session) => ({
      id: session.profile.id,
      kind: session.profile.kind,
      connected: session.runtime.status === "connected",
    }));
    if (!candidates.some((candidate) => candidate.id === sessionId)) {
      const source = sessions.find((session) => session.profile.id === sessionId);
      if (source) {
        candidates.unshift({
          id: source.profile.id,
          kind: source.profile.kind,
          connected: source.runtime.status === "connected",
        });
      }
    }
    return syncInputDispatcherRef.current.enqueue({
      sourceId: sessionId,
      text,
      broadcastEnabled,
      applyAffixes: origin !== "interactive",
      settings,
      candidates,
    }, (targetId, payload) => sendTerminalInput(targetId, payload, origin), () => syncInputRef.current).then((result) => {
      if (!result.failed.length && !result.skipped.length) return;
      const failedNames = result.failed.map((targetId) => (
        sessions.find((session) => session.profile.id === targetId)?.profile.name ?? targetId
      ));
      const details = [
        failedNames.length ? `${failedNames.length} 个目标发送失败：${failedNames.join(", ")}` : "",
        result.skipped.length ? `${result.skipped.length} 个剩余目标已取消` : "",
      ].filter(Boolean).join("；");
      setNotice({
        title: failedNames.length ? "同步输入失败" : "同步输入已停止",
        message: details,
      });
    });
  }

  async function sendTerminalInput(sessionId: string, text: string, origin: SyncInputOrigin) {
    if (!sessionId || !text) return;
    const session = sessions.find((item) => item.profile.id === sessionId);
    if (!session) throw new Error(`unknown session: ${sessionId}`);

    try {
      if (isBackendAvailable()) {
        if (origin === "command") {
          await invokeBackend<SessionEvent>("run_command", { sessionId, command: text });
        } else {
          await invokeBackend<SessionEvent>("send_text", { sessionId, text });
        }
        if (session.profile.connection.kind === "serial") {
          void refreshSerialCapture(sessionId);
        }
      } else {
        const event = createLocalSystemEvent(session.profile, text);
        event.direction = "outbound";
        event.stream = "stdout";
        setLogs((current) => ({ ...current, [sessionId]: [...(current[sessionId] ?? []), event] }));
        if (session.profile.connection.kind === "serial") {
          appendLocalSerialCapture(sessionId, Array.from(new TextEncoder().encode(text)));
        }
      }
    } catch (error) {
      setLogs((current) => ({
        ...current,
        [sessionId]: [...(current[sessionId] ?? []), createLocalSystemEvent(session.profile, `PortMate: send failed: ${formatError(error)}`)],
      }));
      throw error;
    }
  }

  async function completeOneKeyPrompt(
    sessionId: string,
    oneKeyId: string,
    field: OneKeyPromptField,
    promptEventId: string,
  ) {
    await invokeBackend<SessionEvent>("send_one_key", {
      request: {
        id: oneKeyId,
        sessionId,
        field,
        source: "prompt-completion",
        promptEventId,
      },
    });
  }

  async function sendTerminalBytes(sessionId: string, bytes: number[]) {
    if (!sessionId || !bytes.length) return;
    const session = sessions.find((item) => item.profile.id === sessionId);
    if (!session) throw new Error(`unknown session: ${sessionId}`);

    try {
      if (isBackendAvailable()) {
        await invokeBackend<SessionEvent>("send_bytes", { sessionId, bytes });
        if (session.profile.connection.kind === "serial") {
          void refreshSerialCapture(sessionId);
        }
      } else {
        const event = createLocalSystemEvent(session.profile, formatHexBytes(bytes));
        event.direction = "outbound";
        event.stream = "stdout";
        setLogs((current) => ({ ...current, [sessionId]: [...(current[sessionId] ?? []), event] }));
        if (session.profile.connection.kind === "serial") {
          appendLocalSerialCapture(sessionId, bytes);
        }
      }
    } catch (error) {
      setLogs((current) => ({
        ...current,
        [sessionId]: [...(current[sessionId] ?? []), createLocalSystemEvent(session.profile, `PortMate: send failed: ${formatError(error)}`)],
      }));
      throw error;
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
              : sendTerminalInput(target, textPayload, "atomic"),
          ),
        );
        if (index + 1 < Math.max(1, sendCount)) {
          await delay(sendIntervalMs);
        }
      }
      if (sendMode === "text" && textPayload.trim()) {
        rememberCommand(textPayload);
      }
    } catch (error) {
      setNotice({ title: "发送失败", message: formatError(error) });
    } finally {
      setSendBusy(false);
    }
  }

  function runQuickCommand(command: QuickCommand) {
    if (!active) {
      setNotice({ title: "快速命令", message: "请先打开一个终端会话。" });
      return;
    }
    if (command.appendEnter && command.command.trim()) {
      rememberCommand(command.command);
    }
    const dispatch = quickCommandDispatch(command);
    void routeTerminalInput(active.profile.id, dispatch.text, dispatch.origin);
  }

  function rememberCommand(command: string) {
    void import("./command-history-state").then(({ recordCommandHistory }) => {
      setCommandHistoryEntries((current) => recordCommandHistory(current, command, commandHistoryPolicy));
      setCommandHistoryReady(true);
    });
  }

  function clearCommandHistory() {
    setCommandHistoryEntries([]);
    try {
      window.localStorage.removeItem(COMMAND_HISTORY_STORAGE_KEY);
    } catch {
      // Clearing the in-memory list still succeeds when storage is unavailable.
    }
  }

  function setWorkspacePanelVisible(panel: WorkspacePanelId, visible: boolean) {
    setWorkspacePanels((current) => setWorkspacePanelVisibility(current, panel, visible));
    if (visible && workspaceDockPanelIds.includes(panel as WorkspaceDockPanelId)) {
      setWorkspaceDockLayout((current) => activateWorkspaceDockPanel(current, panel as WorkspaceDockPanelId));
    }
  }

  function startWorkspacePanelDrag(event: ReactDragEvent<HTMLElement>, panel: WorkspaceDockPanelId) {
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("application/x-portmate-workspace-panel", panel);
    event.dataTransfer.setData("text/plain", panel);
    setDraggedWorkspacePanel(panel);
  }

  function allowWorkspacePanelDrop(event: ReactDragEvent<HTMLElement>) {
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
  }

  function dropWorkspacePanel(event: ReactDragEvent<HTMLElement>, dock: WorkspaceDockId, index?: number) {
    event.preventDefault();
    event.stopPropagation();
    const transferred = event.dataTransfer.getData("application/x-portmate-workspace-panel")
      || event.dataTransfer.getData("text/plain")
      || draggedWorkspacePanel
      || "";
    if (!workspaceDockPanelIds.includes(transferred as WorkspaceDockPanelId)) return;
    const panel = transferred as WorkspaceDockPanelId;
    setWorkspacePanels((current) => setWorkspacePanelVisibility(current, panel, true));
    setWorkspaceDockLayout((current) => moveWorkspacePanelToDock(current, panel, dock, index));
    setDraggedWorkspacePanel(null);
  }

  function menuToggleState(item: string): boolean | undefined {
    const workspacePanel = workspacePanelMenuItems[item];
    if (workspacePanel) return visibleWorkspacePanels[workspacePanel];
    if (item === "快捷栏") return visibleQuickBar;
    const terminalKeyMode = terminalKeyModeMenuItems[item];
    if (terminalKeyMode) return activeTerminalKeyMode === terminalKeyMode;
    if (item === "同步输入") return syncInput;
    if (item === "块选择") return blockSelection;
    if (item === "专注模式") return focusMode;
    return undefined;
  }

  function requestSessionCredentials(profile: SessionProfile): Promise<ConnectionCredentials | null> {
    if (!isSshLikeProfile(profile)) {
      return Promise.resolve({ username: null, password: null, passphrase: null, oneKeyId: null, savePassword: false, savePassphrase: false });
    }

    const ssh = profile.connection;
    const target = describeProfileEndpoint(profile) || profile.name || "SSH";
    const hasPrivateKey = ssh.identityRefs.some((identity) => Boolean(identity.path) || Boolean(identity.secretRef));
    const prompt: CredentialPromptState = {
      target,
      initialUsername: ssh.username || "",
      oneKeys: sshOneKeysForSession(oneKeys, profile.id),
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

  async function setSerialLine(sessionId: string, line: "dtr" | "rts", value: boolean) {
    const session = sessions.find((item) => item.profile.id === sessionId);
    if (!session || session.profile.connection.kind !== "serial") return;
    try {
      const saved = await invokeBackend<SessionSummary>("serial_set_lines", {
        request: { sessionId, [line]: value },
      });
      setSessions((current) => mergeSessionSummaries(current, saved));
    } catch (error) {
      setNotice({ title: "串口控制失败", message: formatError(error) });
    }
  }

  async function sendSerialBreak(sessionId: string) {
    const session = sessions.find((item) => item.profile.id === sessionId);
    if (!session || session.profile.connection.kind !== "serial") return;
    try {
      await invokeBackend("serial_send_break", { sessionId });
      await refreshActiveLog(sessionId);
    } catch (error) {
      setNotice({ title: "Break 失败", message: formatError(error) });
    }
  }

  function renderWorkspaceDockPanel(panel: WorkspaceDockPanelId) {
    if (panel === "explorer") {
      return (
        <Suspense fallback={null}>
          <LazySessionExplorerPanel
            sessions={sessions}
            activeId={activeId}
            colors={tabColors}
            icons={workspaceUtilityIcons}
            onSelect={activateSession}
            onOpenContextMenu={(event, sessionId) => openAppContextMenu(event, sessionId)}
          />
        </Suspense>
      );
    }
    if (panel === "fileManager") {
      return <FileManagerPanel active={active} transfers={transfers} onTransfer={(task) => setTransfers((current) => mergeTransfers(current, task))} onNotice={setNotice} />;
    }
    if (panel === "history") {
      return (
        <Suspense fallback={null}>
          <LazyCommandHistoryList
            history={commandHistory}
            icons={workspaceUtilityIcons}
            onPick={setSendText}
            beforeList={activeSerial && active ? (
              <SerialMonitorPanel
                key={active.profile.id}
                frames={serialCaptures[active.profile.id] ?? []}
                onOpen={() => void openSerialAnalyzer(active)}
                onClear={() => void clearSerialCapture(active.profile.id)}
                onExport={(frameIds) => void exportSerialCapture(active.profile.id, frameIds)}
                canExport={isBackendAvailable()}
              />
            ) : null}
          />
        </Suspense>
      );
    }
    return (
      <>
        <div className="send-toolbar">
          <button className="send-icon-button" title="发送" aria-label="发送" onClick={() => void runSendPanel()} disabled={sendBusy}>
            <Play size={14} className="green" />
          </button>
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
          {syncInput ? <span className="sync-badge">同步输入 · {syncInputTargetCount} 目标</span> : null}
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
      </>
    );
  }

  const visibleWorkspacePanels = resolveWorkspacePanelVisibility(workspacePanels, focusMode, syncInput);
  const visibleDockPanels = Object.fromEntries(workspaceDockIds.map((dock) => [
    dock,
    visibleWorkspaceDockPanels(workspaceDockLayout, visibleWorkspacePanels, dock),
  ])) as Record<WorkspaceDockId, WorkspaceDockPanelId[]>;
  const activeDockPanels = Object.fromEntries(workspaceDockIds.map((dock) => [
    dock,
    activeWorkspaceDockPanel(workspaceDockLayout, visibleWorkspacePanels, dock),
  ])) as Record<WorkspaceDockId, WorkspaceDockPanelId | null>;
  const visibleQuickBar = quickBarVisible && !focusMode;
  const workspaceLayoutStyle = {
    ...(activeDockPanels.left && workspaceDockSizes.left !== null
      ? { "--workspace-left-size": `min(${workspaceDockSizes.left}px, 38vw)` }
      : {}),
    ...(activeDockPanels.right && workspaceDockSizes.right !== null
      ? { "--workspace-right-size": `min(${workspaceDockSizes.right}px, 38vw)` }
      : {}),
    ...(activeDockPanels.bottom && workspaceDockSizes.bottom !== null
      ? { "--workspace-bottom-size": `min(${workspaceDockSizes.bottom}px, 45vh)` }
      : {}),
  } as CSSProperties;

  return (
    <main className={["wind-root", visibleQuickBar ? "quick-bar-visible" : "", visibleWorkspacePanels.statusBar ? "" : "status-bar-hidden", focusMode ? "focus-mode" : ""].filter(Boolean).join(" ")} onContextMenu={openAppContextMenu} onClick={() => {
      setContextMenu(null);
      setWorkspaceViewContextMenu(null);
    }}>
      <header className="wind-menu">
        <div className="menu-primary">
          <div className="menu-brand" aria-label="PortMate">
            <SquareTerminal size={14} />
            <strong>PortMate</strong>
          </div>
          <div className="menu-row">
            {menuGroups.map((group) => (
              <div key={group.label} className="menu-item" onMouseLeave={() => setOpenMenu(null)}>
                <button type="button" className={openMenu === group.label ? "menu-trigger active" : "menu-trigger"} onClick={() => setOpenMenu(openMenu === group.label ? null : group.label)}>
                  {group.label}
                </button>
                {openMenu === group.label && (
                  <div className="menu-popover">
                    {group.items.map((item) => {
                      const toggleState = menuToggleState(item);
                      const disabled = menuItemDisabled(item, menuCapabilityContext);
                      return (
                        <button
                          type="button"
                          key={item}
                          className={toggleState === undefined ? "" : "menu-toggle"}
                          aria-pressed={toggleState}
                          disabled={disabled}
                          onClick={(event) => {
                            event.stopPropagation();
                            handleMenuAction(item);
                            setOpenMenu(null);
                          }}
                        >
                          <span>{item}</span>
                          {toggleState ? <Check size={13} /> : null}
                        </button>
                      );
                    })}
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>
        <div className="menu-tools">
          <button type="button" title="搜索会话" aria-label="搜索会话" onClick={() => handleMenuAction("会话搜索")}><Search size={13} /></button>
          <button type="button" className={focusMode ? "active" : ""} aria-pressed={focusMode} aria-label={focusMode ? "退出专注模式" : "进入专注模式"} title="专注模式 (Alt+Enter)" onClick={() => setFocusMode((current) => !current)}>
            {focusMode ? <Minimize2 size={12} /> : <Maximize2 size={12} />}
          </button>
        </div>
      </header>

      {visibleQuickBar ? (
        <QuickCommandBar
          commands={quickCommands}
          activeSessionName={active?.profile.name ?? ""}
          onRun={runQuickCommand}
          onManage={() => setUtilityDialog("quick-commands")}
          onClose={() => setQuickBarVisible(false)}
        />
      ) : null}

      <section className={[
        "wind-layout",
        activeDockPanels.left ? "dock-left-visible" : "",
        activeDockPanels.right ? "dock-right-visible" : "",
        activeDockPanels.bottom ? "dock-bottom-visible" : "",
        draggedWorkspacePanel ? "workspace-panel-dragging" : "",
      ].filter(Boolean).join(" ")} style={workspaceLayoutStyle}>
        {workspaceDockIds.map((dock) => {
          const activePanel = activeDockPanels[dock];
          return activePanel ? (
            <WorkspaceDock
              key={dock}
              dock={dock}
              panels={visibleDockPanels[dock]}
              activePanel={activePanel}
              effectiveSize={workspaceDockEffectiveSize(workspaceDockSizes, dock, visibleDockPanels[dock])}
              onActivate={(panel) => setWorkspaceDockLayout((current) => activateWorkspaceDockPanel(current, panel))}
              onClose={(panel) => setWorkspacePanelVisible(panel, false)}
              onResize={(size) => setWorkspaceDockSizes((current) => setWorkspaceDockSize(current, dock, size))}
              onDragStart={startWorkspacePanelDrag}
              onDragEnd={() => setDraggedWorkspacePanel(null)}
              onDragOver={allowWorkspacePanelDrop}
              onDrop={dropWorkspacePanel}
              renderPanel={renderWorkspaceDockPanel}
            />
          ) : null;
        })}

        {draggedWorkspacePanel ? workspaceDockIds.map((dock) => {
          const DockIcon = workspaceDockMeta[dock].icon;
          return (
            <div
              key={dock}
              className={`workspace-dock-drop-target target-${dock}`}
              data-dock-target={dock}
              title={`停靠到${workspaceDockMeta[dock].label}`}
              aria-label={`停靠到${workspaceDockMeta[dock].label}`}
              onDragOver={allowWorkspacePanelDrop}
              onDrop={(event) => dropWorkspacePanel(event, dock)}
            >
              <DockIcon size={18} />
            </div>
          );
        }) : null}

        <section className="center-workspace">
          <TerminalPaneGrid
            root={workspaceRoot}
            sessions={sessions}
            activePaneId={activePaneId}
            zoomedPaneId={zoomedPaneId}
            activeId={activeId}
            eventsBySession={logs}
            oneKeys={oneKeys}
            oneKeyCompletionEnabled={terminalPrefs.oneKeyCompletionEnabled}
            completionSettings={terminalPrefs}
            completionHistory={commandHistory}
            completionQuickCommands={quickCommands}
            mouseReporting={terminalPrefs.mouseReporting}
            copyOnSelect={terminalPrefs.mouseCopyOnSelect}
            blockSelection={blockSelection}
            onInput={(sessionId, text, origin) => void routeTerminalInput(sessionId, text, origin)}
            onOneKeyCompletion={completeOneKeyPrompt}
            onKeyModeChange={(paneId, viewId, keyMode) => setActiveWorkspaceViewKeyMode(keyMode, paneId, viewId)}
            onConnect={(sessionId) => void connectSession(sessionId)}
            onDisconnect={(sessionId) => void disconnectSession(sessionId)}
            onSetSerialLine={(sessionId, line, value) => void setSerialLine(sessionId, line, value)}
            onSendSerialBreak={(sessionId) => void sendSerialBreak(sessionId)}
            onActivate={activateWorkspacePane}
            onCloseView={closeWorkspaceViews}
            onRenameView={openWorkspaceViewRename}
            onOpenViewContextMenu={openWorkspaceViewContextMenu}
            onMoveViewDrop={moveWorkspaceViewToIndex}
            onSplitViewDrop={splitWorkspaceViewFromDrop}
            onSplitRatioChange={(splitId, ratio) => {
              setWorkspaceRoot((current) => updateWorkspaceSplitRatio(current, splitId, ratio));
            }}
          />
        </section>
      </section>

      {visibleWorkspacePanels.statusBar ? <footer className="status-bar">
        {active ? <SysmonApplet key={active.profile.id} session={active} onOpen={() => setUtilityDialog("sysmon")} /> : <span />}
        <span />
        <button
          type="button"
          className={`sync-status terminal-key-mode-status ${syncInput ? "active" : ""}`}
          data-key-mode={activeTerminalKeyMode}
          title="切换远程/本地模式 (Ctrl+Enter)"
          aria-label={`当前${terminalKeyModeLabel(activeTerminalKeyMode)}，切换远程/本地模式`}
          onClick={() => setActiveWorkspaceViewKeyMode(toggleTerminalRemoteLocalMode(activeTerminalKeyMode))}
        >
          {syncInput ? `同步 ${syncInputTargetCount} · ` : ""}{terminalKeyModeLabel(activeTerminalKeyMode)}
        </button>
        {blockSelection ? <span>块选择</span> : null}
        <button type="button" className="status-lock-button" title="锁屏 (Ctrl+Alt+L)" aria-label="锁屏" onClick={() => lockScreen("manual")}>
          <Lock size={12} />
        </button>
      </footer> : null}

      {contextMenu?.kind === "session" && (
        <Suspense fallback={null}>
          <LazySessionContextMenu
            state={contextMenu}
            active={contextSession(contextMenu.sessionId)}
            syncInput={syncInput}
            colors={tabColorChoices}
            onAction={handleContextMenuAction}
            onColor={(color) => {
              setTabColorFromContext(contextMenu.sessionId, color);
              setContextMenu(null);
            }}
          />
        </Suspense>
      )}

      {contextMenu?.kind === "terminal" && (
        <Suspense fallback={null}>
          <LazyTerminalContextMenu
            state={contextMenu}
            onAction={(action) => handleTerminalContextMenuAction(action, contextMenu)}
          />
        </Suspense>
      )}

      {workspaceViewContextMenu && workspaceContextPane && workspaceContextView && workspaceContextSession && (
        <Suspense fallback={null}>
          <LazyWorkspaceViewContextMenu
            state={workspaceViewContextMenu}
            view={workspaceContextView}
            label={workspaceContextView.title || workspaceContextSession.profile.name}
            colors={tabColorChoices}
            canMerge={workspaceContextCanMerge}
            canSwap={workspaceContextCanSwap}
            canZoom={workspacePanes.length > 1}
            {...workspaceContextCapabilities}
            onColor={(color) => changeWorkspaceViewColor(workspaceContextPane.id, workspaceContextView.id, color)}
            onDuplicate={() => {
              setWorkspaceViewContextMenu(null);
              duplicateActiveWorkspaceView(workspaceContextPane.id, workspaceContextView.id);
            }}
            onRename={() => {
              setWorkspaceViewContextMenu(null);
              openWorkspaceViewRename(workspaceContextPane.id, workspaceContextView.id);
            }}
            onAction={(action) => handleWorkspaceViewContextAction(action, workspaceContextPane.id, workspaceContextView.id)}
          />
        </Suspense>
      )}

      {workspaceGroupMove && (
        <WorkspaceGroupMoveDialog
          root={workspaceRoot}
          sourcePaneId={workspaceGroupMove.paneId}
          mode={workspaceGroupMove.mode}
          sessions={sessions}
          onMove={workspaceGroupMove.mode === "group" ? mergeWorkspaceGroup : moveWorkspaceView}
          onClose={() => setWorkspaceGroupMove(null)}
        />
      )}

      {workspaceViewRename && (
        <Suspense fallback={null}>
          <LazyWorkspaceViewRenameDialog
            state={workspaceViewRename}
            onChange={(value) => setWorkspaceViewRename((current) => current ? { ...current, value } : null)}
            onUseSessionName={() => commitWorkspaceViewRename(true)}
            onSave={() => commitWorkspaceViewRename(false)}
            onClose={closeWorkspaceViewRename}
          />
        </Suspense>
      )}

      {dialog === "terminal" && (
        <TerminalSettingsDialog
          initialPrefs={terminalPrefs}
          sessions={sessions}
          syncSettings={syncInputSettings}
          workspaceKeymap={workspaceKeymap}
          onPrefsChange={setTerminalPrefs}
          onClearCommandHistory={clearCommandHistory}
          onSyncSettingsChange={setSyncInputSettings}
          onWorkspaceKeymapChange={setWorkspaceKeymap}
          onClose={() => setDialog(null)}
        />
      )}
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
      {utilityDialog === "tmux" && active && (
        <Suspense fallback={null}>
          <LazyTmuxDialog session={active} onClose={() => setUtilityDialog(null)} onDone={(message) => {
            setUtilityDialog(null);
            setNotice({ title: "Tmux", message });
            void refreshActiveLog(active.profile.id);
          }} />
        </Suspense>
      )}
      {utilityDialog === "sysmon" && active && <SysmonDialog key={active.profile.id} session={active} onClose={() => setUtilityDialog(null)} />}
      {utilityDialog === "search" && (
        <Suspense fallback={null}>
          <LazySearchDialog state={searchDialog} sessions={sessions} logs={logs} onChange={setSearchDialog} onSelect={(sessionId) => {
            activateSession(sessionId);
            setUtilityDialog(null);
          }} onClose={() => setUtilityDialog(null)} />
        </Suspense>
      )}
      {utilityDialog === "logs" && <LogManagerDialog sessions={sessions} activeId={activeId} onClose={() => setUtilityDialog(null)} onNotice={(message) => setNotice({ title: "日志管理", message })} />}
      {utilityDialog === "keys" && <KeyManagerDialog hostKeys={hostKeys} sessions={sessions} onChange={setHostKeys} onProfileChange={applySavedSession} onProfilesChange={applySavedSessions} onClose={() => setUtilityDialog(null)} />}
      {utilityDialog === "mcp" && (
        <Suspense fallback={null}>
          <LazyMcpDialog grants={grants} audit={audit} sessions={sessions} onClose={() => setUtilityDialog(null)} onGrantChange={setGrants} onAuditChange={setAudit} />
        </Suspense>
      )}
      {utilityDialog === "one-keys" && (
        <Suspense fallback={null}>
          <LazyOneKeyDialog oneKeys={oneKeys} sessions={sessions} activeId={activeId} onChange={setOneKeys} onClose={() => setUtilityDialog(null)} />
        </Suspense>
      )}
      {utilityDialog === "quick-commands" && (
        <Suspense fallback={null}>
          <LazyQuickCommandDialog
            commands={quickCommands}
            onSave={(items) => {
              setQuickCommands(items);
              setUtilityDialog(null);
              if (items.length) setQuickBarVisible(true);
            }}
            onClose={() => setUtilityDialog(null)}
          />
        </Suspense>
      )}
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
      {!screenLock && activeMcpApproval && (
        <Suspense fallback={null}>
          <LazyMcpApprovalDialog
            key={activeMcpApproval.id}
            request={activeMcpApproval}
            sessionName={approvalSessionName}
            queueCount={mcpApprovals.length}
            onDecision={respondMcpApproval}
            onExpired={expireMcpApproval}
          />
        </Suspense>
      )}
      {screenLock && (
        <ScreenLockOverlay
          state={screenLock}
          onUnlock={unlockScreen}
          onRetry={retryPrepareScreenLock}
        />
      )}
    </main>
  );
}

function ScreenLockOverlay({
  state,
  onUnlock,
  onRetry,
}: {
  state: NonNullable<ScreenLockState>;
  onUnlock: (password?: string) => Promise<void>;
  onRetry: () => void;
}) {
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const overlayRef = useRef<HTMLDivElement | null>(null);
  const primaryRef = useRef<HTMLInputElement | HTMLButtonElement | null>(null);

  useEffect(() => {
    const overlay = overlayRef.current;
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const siblings = overlay?.parentElement
      ? [...overlay.parentElement.children].filter((element): element is HTMLElement => element instanceof HTMLElement && element !== overlay)
      : [];
    const previousInert = siblings.map((element) => element.inert);
    siblings.forEach((element) => {
      element.inert = true;
    });
    return () => {
      siblings.forEach((element, index) => {
        element.inert = previousInert[index];
      });
      if (previousFocus?.isConnected) previousFocus.focus({ preventScroll: true });
    };
  }, []);

  useEffect(() => {
    setPassword("");
    setError("");
    window.requestAnimationFrame(() => primaryRef.current?.focus({ preventScroll: true }));
  }, [state.mode]);

  async function submit(event?: FormEvent) {
    event?.preventDefault();
    if (busy || state.mode === "preparing" || state.mode === "error") return;
    setBusy(true);
    setError("");
    try {
      await onUnlock(password);
    } catch (unlockError) {
      setPassword("");
      setError(formatError(unlockError));
      window.requestAnimationFrame(() => primaryRef.current?.focus({ preventScroll: true }));
    } finally {
      setBusy(false);
    }
  }

  function trapFocus(event: React.KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      return;
    }
    if (event.key !== "Tab") return;
    const controls = [...(overlayRef.current?.querySelectorAll<HTMLElement>("input:not(:disabled), button:not(:disabled)") ?? [])];
    if (!controls.length) {
      event.preventDefault();
      return;
    }
    const currentIndex = controls.indexOf(document.activeElement as HTMLElement);
    const nextIndex = event.shiftKey
      ? (currentIndex <= 0 ? controls.length - 1 : currentIndex - 1)
      : (currentIndex < 0 || currentIndex === controls.length - 1 ? 0 : currentIndex + 1);
    event.preventDefault();
    controls[nextIndex].focus();
  }

  const reasonLabel = state.reason === "idle"
    ? "空闲超时"
    : state.reason === "startup" ? "启动保护" : state.reason === "restored" ? "刷新后恢复" : "手动锁定";

  return (
    <div
      ref={overlayRef}
      className="screen-lock-overlay"
      role="dialog"
      aria-modal="true"
      aria-labelledby="screen-lock-title"
      onKeyDown={trapFocus}
    >
      <form className="screen-lock-panel" onSubmit={(event) => void submit(event)}>
        <div className="screen-lock-brand">
          <span className="screen-lock-icon"><Lock size={20} /></span>
          <span>PortMate</span>
        </div>
        <div className="screen-lock-heading">
          <h1 id="screen-lock-title">屏幕已锁定</h1>
          <span>{reasonLabel} · {new Date(state.lockedAt).toLocaleTimeString()}</span>
        </div>
        <div className="screen-lock-rule" />
        {state.mode === "preparing" ? (
          <div className="screen-lock-progress" role="status">
            <LoaderCircle size={17} />
            <span>正在保护凭据</span>
          </div>
        ) : null}
        {state.mode === "vault" ? (
          <>
            <label className="screen-lock-field">
              <span>Portable Vault 主密码</span>
              <input
                ref={(element) => { primaryRef.current = element; }}
                type="password"
                value={password}
                autoComplete="current-password"
                disabled={busy}
                onChange={(event) => setPassword(event.target.value)}
              />
            </label>
            <button className={busy ? "screen-lock-primary busy" : "screen-lock-primary"} type="submit" disabled={busy || !password}>
              {busy ? <LoaderCircle size={15} /> : <Unlock size={15} />}
              <span>{busy ? "验证中" : "解锁"}</span>
            </button>
          </>
        ) : null}
        {state.mode === "confirm" ? (
          <>
            <p className="screen-lock-message">{state.message}</p>
            <button
              ref={(element) => { primaryRef.current = element; }}
              className="screen-lock-primary"
              type="button"
              disabled={busy}
              onClick={() => void submit()}
            >
              <Unlock size={15} />
              <span>返回工作台</span>
            </button>
          </>
        ) : null}
        {state.mode === "error" ? (
          <>
            <p className="screen-lock-error" role="alert">{state.message}</p>
            <button
              ref={(element) => { primaryRef.current = element; }}
              className="screen-lock-retry"
              type="button"
              onClick={onRetry}
            >
              <RefreshCw size={15} />
              <span>重试凭据检查</span>
            </button>
          </>
        ) : null}
        {error ? <p className="screen-lock-error" role="alert">{error}</p> : null}
      </form>
    </div>
  );
}

function WorkspaceDock({
  dock,
  panels,
  activePanel,
  effectiveSize,
  onActivate,
  onClose,
  onResize,
  onDragStart,
  onDragEnd,
  onDragOver,
  onDrop,
  renderPanel,
}: {
  dock: WorkspaceDockId;
  panels: WorkspaceDockPanelId[];
  activePanel: WorkspaceDockPanelId;
  effectiveSize: number;
  onActivate: (panel: WorkspaceDockPanelId) => void;
  onClose: (panel: WorkspaceDockPanelId) => void;
  onResize: (size: number | null) => void;
  onDragStart: (event: ReactDragEvent<HTMLElement>, panel: WorkspaceDockPanelId) => void;
  onDragEnd: () => void;
  onDragOver: (event: ReactDragEvent<HTMLElement>) => void;
  onDrop: (event: ReactDragEvent<HTMLElement>, dock: WorkspaceDockId, index?: number) => void;
  renderPanel: (panel: WorkspaceDockPanelId) => React.ReactNode;
}) {
  const limits = workspaceDockSizeLimits[dock];
  const resizeLabel = dock === "bottom" ? "调整底部停靠区高度" : `调整${dock === "left" ? "左侧" : "右侧"}停靠区宽度`;

  function updateDockSizeFromPointer(event: ReactPointerEvent<HTMLButtonElement>) {
    const layout = event.currentTarget.closest(".wind-layout")?.getBoundingClientRect();
    if (!layout) return;
    const requested = dock === "left"
      ? event.clientX - layout.left
      : dock === "right"
        ? layout.right - event.clientX
        : layout.bottom - event.clientY;
    const viewportLimit = dock === "bottom" ? layout.height * 0.45 : layout.width * 0.38;
    onResize(clampWorkspaceDockSize(dock, Math.min(requested, viewportLimit)));
  }

  function handleDockSizeKey(event: React.KeyboardEvent<HTMLButtonElement>) {
    const decrease = dock === "left"
      ? event.key === "ArrowLeft"
      : dock === "right"
        ? event.key === "ArrowRight"
        : event.key === "ArrowDown";
    const increase = dock === "left"
      ? event.key === "ArrowRight"
      : dock === "right"
        ? event.key === "ArrowLeft"
        : event.key === "ArrowUp";
    let nextSize: number | null = decrease ? effectiveSize - 16 : increase ? effectiveSize + 16 : null;
    if (event.key === "Home") nextSize = limits.min;
    if (event.key === "End") nextSize = limits.max;
    if (nextSize === null) return;
    event.preventDefault();
    onResize(nextSize);
  }

  return (
    <aside className={`workspace-dock workspace-dock-${dock}`} data-dock={dock} data-active-panel={activePanel}>
      <div
        className="workspace-dock-windows"
        aria-label={`${workspaceDockMeta[dock].label}停靠区`}
        data-panel-count={panels.length}
        onDragOver={(event) => {
          if (event.target === event.currentTarget) onDragOver(event);
        }}
        onDrop={(event) => {
          if (event.target === event.currentTarget) onDrop(event, dock, panels.length);
        }}
      >
        {panels.map((panel, index) => {
          const metadata = workspaceDockPanelMeta[panel];
          const PanelIcon = metadata.icon;
          const active = panel === activePanel;
          return (
            <section
              key={panel}
              className={active ? "workspace-panel-window active" : "workspace-panel-window"}
              data-panel={panel}
              onPointerDownCapture={() => onActivate(panel)}
            >
              <header
                className="workspace-dock-tab"
                draggable
                data-panel={panel}
                onDragStart={(event) => onDragStart(event, panel)}
                onDragEnd={(event) => {
                  delete event.currentTarget.dataset.panelDropPosition;
                  onDragEnd();
                }}
                onDragOver={(event) => {
                  event.stopPropagation();
                  onDragOver(event);
                  const bounds = event.currentTarget.getBoundingClientRect();
                  const after = dock === "bottom"
                    ? event.clientX >= bounds.left + bounds.width / 2
                    : event.clientY >= bounds.top + bounds.height / 2;
                  event.currentTarget.dataset.panelDropPosition = after ? "after" : "before";
                }}
                onDragLeave={(event) => {
                  if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
                    delete event.currentTarget.dataset.panelDropPosition;
                  }
                }}
                onDrop={(event) => {
                  const after = event.currentTarget.dataset.panelDropPosition === "after";
                  delete event.currentTarget.dataset.panelDropPosition;
                  onDrop(event, dock, index + (after ? 1 : 0));
                }}
              >
                <button
                  type="button"
                  className="workspace-dock-tab-label"
                  aria-pressed={active}
                  title={`聚焦${metadata.label}`}
                  onClick={() => onActivate(panel)}
                >
                  <PanelIcon size={13} />
                  <span>{metadata.label}</span>
                </button>
                <button
                  type="button"
                  className="workspace-dock-tab-close"
                  title={`隐藏${metadata.label}`}
                  aria-label={`隐藏${metadata.label}`}
                  onClick={() => onClose(panel)}
                >
                  <X size={12} />
                </button>
              </header>
              <section
                className={`workspace-dock-content panel-${panel}`}
                data-panel={panel}
                role="region"
                aria-label={metadata.label}
              >
                {renderPanel(panel)}
              </section>
            </section>
          );
        })}
      </div>
      <button
        type="button"
        className="workspace-dock-resizer"
        role="separator"
        aria-label={resizeLabel}
        aria-orientation={dock === "bottom" ? "horizontal" : "vertical"}
        aria-valuemin={limits.min}
        aria-valuemax={limits.max}
        aria-valuenow={effectiveSize}
        title={`${resizeLabel}，双击复位`}
        onPointerDown={(event) => {
          event.currentTarget.setPointerCapture(event.pointerId);
        }}
        onPointerMove={(event) => {
          if (event.currentTarget.hasPointerCapture(event.pointerId)) updateDockSizeFromPointer(event);
        }}
        onPointerUp={(event) => {
          if (event.currentTarget.hasPointerCapture(event.pointerId)) {
            event.currentTarget.releasePointerCapture(event.pointerId);
          }
        }}
        onPointerCancel={(event) => {
          if (event.currentTarget.hasPointerCapture(event.pointerId)) {
            event.currentTarget.releasePointerCapture(event.pointerId);
          }
        }}
        onDoubleClick={() => onResize(null)}
        onKeyDown={handleDockSizeKey}
      />
    </aside>
  );
}

function QuickCommandBar({
  commands,
  activeSessionName,
  onRun,
  onManage,
  onClose,
}: {
  commands: QuickCommand[];
  activeSessionName: string;
  onRun: (command: QuickCommand) => void;
  onManage: () => void;
  onClose: () => void;
}) {
  return (
    <nav className="quick-command-bar" aria-label="快速命令栏">
      <strong>快速命令</strong>
      <div className="quick-command-strip">
        {commands.length ? commands.map((command) => (
          <button
            key={command.id}
            type="button"
            className="quick-command-run"
            title={`${command.label} · ${command.appendEnter ? "执行" : "插入"}\n${command.command}`}
            aria-label={`${command.appendEnter ? "执行" : "插入"}快速命令 ${command.label}`}
            disabled={!activeSessionName}
            onClick={() => onRun(command)}
          >
            {command.appendEnter ? <Play size={11} /> : <Pencil size={11} />}
            <span>{command.label}</span>
          </button>
        )) : (
          <button type="button" className="quick-command-empty" onClick={onManage}><Plus size={12} /><span>添加命令</span></button>
        )}
      </div>
      <span className="quick-command-target" title={activeSessionName || "未打开会话"}>{activeSessionName || "未打开会话"}</span>
      <button type="button" className="quick-command-tool" title="管理快速命令" aria-label="管理快速命令" onClick={onManage}><Settings size={14} /></button>
      <button type="button" className="quick-command-tool" title="隐藏快捷栏" aria-label="隐藏快捷栏" onClick={onClose}><X size={14} /></button>
    </nav>
  );
}

function SerialMonitorPanel({
  frames,
  onOpen,
  onClear,
  onExport,
  canExport,
}: {
  frames: SerialCaptureFrame[];
  onOpen: () => void;
  onClear: () => void;
  onExport: (frameIds: string[]) => void;
  canExport: boolean;
}) {
  const [direction, setDirection] = useState<SerialCaptureDirectionFilter>("all");
  const [query, setQuery] = useState("");
  const visible = useMemo(
    () => filterSerialCaptureFrames(frames, direction, query),
    [frames, direction, query],
  );

  return (
    <div className="serial-monitor">
      <div className="serial-monitor-controls">
        <div className="serial-monitor-filters" aria-label="串口捕获方向">
          {(["all", "inbound", "outbound"] as const).map((value) => (
            <button
              type="button"
              key={value}
              aria-pressed={direction === value}
              onClick={() => setDirection(value)}
            >
              {value === "all" ? "全部" : value === "inbound" ? "RX" : "TX"}
            </button>
          ))}
          <span>{visible.length}/{frames.length}</span>
        </div>
        <div className="serial-monitor-search">
          <Search size={13} />
          <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Hex / ASCII" aria-label="筛选串口捕获" />
          <button type="button" title="在独立窗口中分析" aria-label="打开串口分析器" onClick={onOpen}>
            <Maximize2 size={13} />
          </button>
          <button
            type="button"
            title="导出可见帧（原始字节，不脱敏）"
            aria-label="导出可见串口帧"
            disabled={!canExport || !visible.length}
            onClick={() => onExport(visible.map((frame) => frame.id))}
          >
            <Download size={13} />
          </button>
          <button type="button" title="清空串口捕获" aria-label="清空串口捕获" disabled={!frames.length} onClick={onClear}>
            <Trash2 size={13} />
          </button>
        </div>
      </div>
      <div className="serial-monitor-list">
        {!visible.length ? <div className="empty-pane top">没有匹配的串口帧</div> : null}
        {visible.slice().reverse().map((frame) => (
          <div key={frame.id} className={`serial-monitor-row ${frame.direction}`}>
            <div className="serial-monitor-meta">
              <span>{formatEventClock(frame.ts)}</span>
              <span>{frame.truncated ? `${frame.bytes.length}/${frame.originalLength} B` : `${frame.originalLength} B`}</span>
              <strong>{frame.direction === "inbound" ? "RX" : "TX"}</strong>
            </div>
            <code>{serialCaptureHex(frame.bytes) || "--"}</code>
            <small>{serialCaptureAscii(frame.bytes)}</small>
          </div>
        ))}
      </div>
    </div>
  );
}

function TransferList({ transfers, onRetry, onCancel }: { transfers: TransferTask[]; onRetry: (task: TransferTask) => void; onCancel: (task: TransferTask) => void }) {
  if (!transfers.length) return <div className="empty-pane top">没有传输任务</div>;
  return (
    <div className="transfer-list">
      {transfers.slice().reverse().map((task) => {
        const message = transferDisplayMessage(task);
        const StatusIcon = task.status === "queued" ? Clock3
          : task.status === "running" ? LoaderCircle
            : task.status === "completed" ? CheckCircle2
              : task.status === "cancelled" ? Ban
                : AlertCircle;
        return (
          <div key={task.id} className={`transfer-row status-${task.status}`}>
            <div className="transfer-row-head">
              <strong>{task.protocol}</strong>
              <span className="transfer-status"><StatusIcon size={14} /><span>{transferStatusLabel(task.status)}</span></span>
              <div className="transfer-row-actions">
                {task.status === "running" ? (
                  <button type="button" onClick={() => onCancel(task)}>取消</button>
                ) : null}
                {task.status === "failed" || task.status === "cancelled" ? (
                  <button type="button" onClick={() => onRetry(task)}>重试</button>
                ) : null}
                {task.status === "failed" ? (
                  <button
                    className="transfer-icon-button"
                    type="button"
                    title="复制失败诊断"
                    aria-label="复制失败诊断"
                    onClick={() => void navigator.clipboard?.writeText(transferDiagnosticText(task)).catch(() => {})}
                  >
                    <Copy size={14} />
                  </button>
                ) : null}
              </div>
            </div>
            <small title={`${task.source} → ${task.destination}`}>{task.source} → {task.destination}</small>
            <small>
              {formatBytes(task.bytesDone)} / {task.bytesTotal ? formatBytes(task.bytesTotal) : "未知"}
              {task.averageBytesPerSecond ? ` · ${formatBytes(task.averageBytesPerSecond)}/s` : ""}
              {task.startedAt && task.finishedAt ? ` · ${formatDuration(task.startedAt, task.finishedAt)}` : ""}
              {task.status === "failed" && task.finishedAt ? ` · ${formatEventClock(task.finishedAt)}` : ""}
            </small>
            {message ? <div className="transfer-message" role={task.status === "failed" ? "alert" : undefined} title={message}><AlertCircle size={14} /><span>{message}</span></div> : null}
            <div className="transfer-progress">
              <span style={{ width: `${task.bytesTotal ? Math.min(100, (task.bytesDone / task.bytesTotal) * 100) : task.status === "completed" ? 100 : 0}%` }} />
            </div>
          </div>
        );
      })}
    </div>
  );
}

type FilePanelState = {
  path: string;
  entries: FileEntry[];
  selected: FileEntry[];
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
  entries: FileEntry[];
} | null;

type TransferConflictPolicy = "fail" | "overwrite" | "skip" | "rename";
type FileSelectionModifiers = { shiftKey: boolean; ctrlKey: boolean; metaKey: boolean };

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
  const [localPanel, setLocalPanel] = useState<FilePanelState>(() => ({ path: defaultLocalPath(), entries: [], selected: [], busy: false, error: "" }));
  const [remotePanel, setRemotePanel] = useState<FilePanelState>(() => ({ path: ".", entries: [], selected: [], busy: false, error: "" }));
  const [propertiesDialog, setPropertiesDialog] = useState<FilePropertiesDialogState>(null);
  const [draggedFile, setDraggedFile] = useState<FileDragState>(null);
  const [dropTarget, setDropTarget] = useState<boolean | null>(null);
  const [externalDrop, setExternalDrop] = useState<ExternalDropState>(null);
  const [conflictPolicy, setConflictPolicy] = useState<TransferConflictPolicy>("fail");
  const selectionAnchors = useRef<{ local: string; remote: string }>({ local: "", remote: "" });
  const fileLoadEpochs = useRef({ local: 0, remote: 0 });
  const activeFileSessionIdRef = useRef("");
  const canRemote = Boolean(active && isSshLikeProfile(active.profile) && active.runtime.status === "connected");
  activeFileSessionIdRef.current = canRemote ? active?.profile.id ?? "" : "";

  useEffect(() => {
    void loadFiles(false);
  }, []);

  useEffect(() => {
    if (canRemote) {
      setRemotePanel((current) => ({ ...current, path: ".", entries: [], selected: [], error: "" }));
      void loadFiles(true, ".");
    } else {
      fileLoadEpochs.current.remote += 1;
      setRemotePanel((current) => ({ ...current, entries: [], selected: [], error: "" }));
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
  }, [canRemote, active?.profile.id, localPanel.path, remotePanel.path, conflictPolicy]);

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

  function selectFileEntry(remote: boolean, entry: FileEntry, event: FileSelectionModifiers) {
    const setter = remote ? setRemotePanel : setLocalPanel;
    const anchorKey = remote ? "remote" : "local";
    setter((current) => {
      const result = updateFileSelection(
        current.entries,
        current.selected,
        entry,
        selectionAnchors.current[anchorKey],
        event,
      );
      selectionAnchors.current[anchorKey] = result.anchorPath;
      return { ...current, selected: result.selected };
    });
  }

  function selectAllFileEntries(remote: boolean) {
    const setter = remote ? setRemotePanel : setLocalPanel;
    setter((current) => ({
      ...current,
      selected: current.selected.length === current.entries.length ? [] : [...current.entries],
    }));
  }

  async function loadFiles(remote: boolean, nextPath = remote ? remotePanel.path : localPanel.path) {
    const loadKey = remote ? "remote" : "local";
    const epoch = fileLoadEpochs.current[loadKey] + 1;
    fileLoadEpochs.current[loadKey] = epoch;
    const sessionId = remote ? active?.profile.id ?? "" : "";
    if (remote && (!canRemote || !sessionId)) return;
    updatePanel(remote, { busy: true, error: "" });
    try {
      const nextEntries = await invokeBackend<FileEntry[]>("list_files", { request: { sessionId: sessionId || null, path: nextPath, remote } });
      if (fileLoadEpochs.current[loadKey] !== epoch
        || (remote && activeFileSessionIdRef.current !== sessionId)) return;
      updatePanel(remote, { entries: nextEntries, path: nextPath, selected: [] });
      selectionAnchors.current[remote ? "remote" : "local"] = "";
    } catch (error) {
      if (fileLoadEpochs.current[loadKey] !== epoch
        || (remote && activeFileSessionIdRef.current !== sessionId)) return;
      updatePanel(remote, { entries: [], error: formatError(error) });
    } finally {
      if (fileLoadEpochs.current[loadKey] === epoch
        && (!remote || activeFileSessionIdRef.current === sessionId)) {
        updatePanel(remote, { busy: false });
      }
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
    if (!panel.selected.length) return;
    if (!window.confirm(`删除选中的 ${panel.selected.length} 项?`)) return;
    try {
      for (const entry of panel.selected) {
        await invokeBackend("delete_path", { request: { sessionId: active?.profile.id ?? null, path: entry.path, remote } });
      }
      await loadFiles(remote, panel.path);
    } catch (error) {
      updatePanel(remote, { error: formatError(error) });
    }
  }

  async function renameSelected(remote: boolean) {
    const panel = remote ? remotePanel : localPanel;
    const selected = panel.selected[0];
    if (panel.selected.length !== 1 || !selected) return;
    const nextName = window.prompt("新名称", selected.name);
    if (!nextName?.trim()) return;
    const nextPath = joinFilePath(parentPath(selected.path, remote), nextName.trim(), remote);
    try {
      await invokeBackend("rename_path", { request: { sessionId: active?.profile.id ?? null, oldPath: selected.path, newPath: nextPath, remote } });
      await loadFiles(remote, panel.path);
    } catch (error) {
      updatePanel(remote, { error: formatError(error) });
    }
  }

  async function chmodSelected(remote: boolean) {
    const panel = remote ? remotePanel : localPanel;
    const selected = panel.selected[0];
    if (panel.selected.length !== 1 || !selected) return;
    const modeText = window.prompt("八进制权限", "0644");
    if (!modeText?.trim()) return;
    const mode = Number.parseInt(modeText.replace(/^0o/i, ""), 8);
    if (!Number.isFinite(mode)) return;
    try {
      await invokeBackend("chmod_path", { request: { sessionId: active?.profile.id ?? null, path: selected.path, mode, remote } });
      await loadFiles(remote, panel.path);
    } catch (error) {
      updatePanel(remote, { error: formatError(error) });
    }
  }

  async function showProperties(remote: boolean) {
    const panel = remote ? remotePanel : localPanel;
    const selected = panel.selected[0];
    if (panel.selected.length !== 1 || !selected) return;
    const nextState: NonNullable<FilePropertiesDialogState> = { remote, path: selected.path, properties: null, busy: true, error: "" };
    setPropertiesDialog(nextState);
    try {
      const properties = await invokeBackend<FileProperties>("file_properties", { request: { sessionId: active?.profile.id ?? null, path: selected.path, remote } });
      setPropertiesDialog({ ...nextState, properties, busy: false });
    } catch (error) {
      setPropertiesDialog({ ...nextState, busy: false, error: formatError(error) });
    }
  }

  async function transferBetween(upload: boolean) {
    if (!active || !canRemote) return;
    const selected = upload ? localPanel.selected : remotePanel.selected;
    if (!selected.length) return;
    await queueFileBatch(
      !upload,
      selected,
      upload,
      upload ? remotePanel.path : localPanel.path,
      upload ? "批量上传" : "批量下载",
    );
  }

  async function queueFileBatch(
    sourceRemote: boolean,
    entries: FileEntry[],
    destinationRemote: boolean,
    destination: string,
    title: string,
  ) {
    if (!active || !canRemote || !entries.length) return;
    setExternalDrop({
      remote: destinationRemote,
      taskIds: [],
      message: `正在规划 ${entries.length} 个选中项`,
      status: "planning",
    });
    updatePanel(destinationRemote, { busy: true, error: "" });
    try {
      const result = await invokeBackend<ExternalDropResult>("start_file_batch", {
        request: {
          sessionId: active.profile.id,
          paths: entries.map((entry) => entry.path),
          sourceRemote,
          destination,
          destinationRemote,
          conflictPolicy,
        },
      });
      result.tasks.forEach(onTransfer);
      const parts = [
        `${result.tasks.length} 个文件`,
        formatBytes(result.totalBytes),
        `${result.directoriesPrepared} 个新目录`,
      ];
      if (result.skipped.length) parts.push(`跳过 ${result.skipped.length} 项`);
      const message = parts.join(" · ");
      setExternalDrop({
        remote: destinationRemote,
        taskIds: result.tasks.map((task) => task.id),
        message,
        status: result.tasks.length ? "queued" : result.skipped.length ? "warning" : "completed",
      });
      onNotice({ title, message });
      if (!result.tasks.length) {
        await loadFiles(destinationRemote, destination);
      }
    } catch (error) {
      const message = formatError(error);
      setExternalDrop(null);
      updatePanel(destinationRemote, { error: message });
      onNotice({ title: `${title}失败`, message });
    } finally {
      updatePanel(destinationRemote, { busy: false });
    }
  }

  function startFileDrag(remote: boolean, entry: FileEntry, event: ReactDragEvent<HTMLElement>) {
    if (!canRemote) return;
    const panel = remote ? remotePanel : localPanel;
    const entries = panel.selected.some((item) => item.path === entry.path) ? panel.selected : [entry];
    setDraggedFile({ remote, entries });
    event.dataTransfer.effectAllowed = "copy";
    event.dataTransfer.setData("application/x-portmate-file", JSON.stringify({ remote, paths: entries.map((item) => item.path) }));
  }

  function handleDragOver(remote: boolean, event: ReactDragEvent<HTMLElement>) {
    if (!canRemote || !draggedFile || draggedFile.remote === remote) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
    setDropTarget(remote);
  }

  async function dropFile(remote: boolean, event: ReactDragEvent<HTMLElement>) {
    event.preventDefault();
    const dropped = draggedFile;
    setDropTarget(null);
    setDraggedFile(null);
    if (!active || !canRemote || !dropped || dropped.remote === remote) return;
    const targetPanel = remote ? remotePanel : localPanel;
    await queueFileBatch(dropped.remote, dropped.entries, remote, targetPanel.path, "拖拽传输");
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
          conflictPolicy,
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
    const selected = panel.selected[0];
    if (!selected || selected.isDir) return;
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
          conflictPolicy={conflictPolicy}
          onConflictPolicyChange={setConflictPolicy}
          onSelect={(entry, event) => selectFileEntry(false, entry, event)}
          onSelectAll={() => selectAllFileEntries(false)}
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
            conflictPolicy={conflictPolicy}
            onConflictPolicyChange={setConflictPolicy}
            onSelect={(entry, event) => selectFileEntry(true, entry, event)}
            onSelectAll={() => selectAllFileEntries(true)}
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
  conflictPolicy,
  onPathChange,
  onLoad,
  onSelect,
  onSelectAll,
  onConflictPolicyChange,
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
  conflictPolicy: TransferConflictPolicy;
  onPathChange: (path: string) => void;
  onLoad: (path: string) => void;
  onSelect: (entry: FileEntry, event: FileSelectionModifiers) => void;
  onSelectAll: () => void;
  onConflictPolicyChange: (policy: TransferConflictPolicy) => void;
  onDragStart: (entry: FileEntry, event: ReactDragEvent<HTMLElement>) => void;
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
        <input aria-label={`${title}路径`} value={panel.path} onChange={(event) => onPathChange(event.target.value)} onKeyDown={(event) => {
          if (event.key === "Enter") {
            onLoad(panel.path);
          }
        }} />
        <button type="button" title={`刷新${title}目录`} aria-label={`刷新${title}目录`} onClick={() => onLoad(panel.path)} disabled={panel.busy}><RefreshCw size={13} /></button>
      </div>
      <div className="file-actions">
        <button type="button" title={panel.selected.length === panel.entries.length && panel.entries.length ? "清除选择" : "全选"} aria-label={panel.selected.length === panel.entries.length && panel.entries.length ? "清除选择" : "全选"} onClick={onSelectAll}><ListChecks size={13} /></button>
        <button type="button" title="新建文件夹" aria-label="新建文件夹" onClick={onCreateDir}><FolderPlus size={13} /></button>
        <button type="button" title="重命名" aria-label="重命名" onClick={onRename} disabled={panel.selected.length !== 1}><Pencil size={13} /></button>
        <button type="button" title="删除" aria-label="删除" onClick={onDelete} disabled={!panel.selected.length}><Trash2 size={13} /></button>
        <button type="button" title="修改权限" aria-label="修改权限" onClick={onChmod} disabled={panel.selected.length !== 1}><ShieldCheck size={13} /></button>
        <button type="button" title="文件属性" aria-label="文件属性" onClick={onProperties} disabled={panel.selected.length !== 1}><Info size={13} /></button>
        <select value={conflictPolicy} onChange={(event) => onConflictPolicyChange(event.target.value as TransferConflictPolicy)} aria-label="文件冲突策略" title="文件冲突策略">
          <option value="fail">停止</option>
          <option value="overwrite">覆盖</option>
          <option value="skip">跳过</option>
          <option value="rename">重命名</option>
        </select>
        <button type="button" title={transferLabel} aria-label={`${transferLabel}${panel.selected.length > 1 ? ` ${panel.selected.length} 项` : ""}`} onClick={onTransfer} disabled={!panel.selected.length || !canTransfer}>
          {remote ? <Download size={13} /> : <Upload size={13} />}
        </button>
      </div>
      {panel.error ? (
        <div className="file-error">{panel.error}</div>
      ) : dropStatus ? (
        <div className={`file-pane-status ${dropStatus.status}`}>{dropStatus.message}</div>
      ) : null}
      <div className="file-list" role="listbox" aria-multiselectable="true">
        <button className="file-row up" onClick={() => onLoad(parentPath(panel.path, remote))}>
          <span className="file-row-check" />
          <Folder size={13} />
          <span>..</span>
          <small />
        </button>
        {panel.entries.map((entry) => (
          <div
            key={entry.path}
            className={panel.selected.some((item) => item.path === entry.path) ? "file-row active" : "file-row"}
            role="option"
            aria-selected={panel.selected.some((item) => item.path === entry.path)}
            tabIndex={0}
            draggable={canTransfer}
            onDragStart={(event) => onDragStart(entry, event)}
            onDragEnd={onDragEnd}
            onClick={(event) => onSelect(entry, event)}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                onSelect(entry, event);
              }
            }}
            onDoubleClick={() => {
              if (entry.isDir) {
                onLoad(entry.path);
              }
            }}
          >
            <input type="checkbox" tabIndex={-1} readOnly checked={panel.selected.some((item) => item.path === entry.path)} aria-label={`选择 ${entry.name}`} />
            {entry.isDir ? <Folder size={13} /> : <File size={13} />}
            <span>{entry.name}</span>
            <small>{entry.isDir ? "dir" : formatBytes(entry.size)}</small>
          </div>
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

function WorkspaceGroupMoveDialog({
  root,
  sourcePaneId,
  mode,
  sessions,
  onMove,
  onClose,
}: {
  root: WorkspaceNode | null;
  sourcePaneId: string;
  mode: "view" | "group";
  sessions: SessionSummary[];
  onMove: (sourcePaneId: string, targetPaneId: string) => void;
  onClose: () => void;
}) {
  const groups = workspacePaneLeaves(root);
  const source = groups.find((pane) => pane.id === sourcePaneId);
  const sourceView = source ? workspacePaneActiveView(source) : undefined;
  const sourceSession = sessions.find((session) => session.profile.id === sourceView?.sessionId);
  const targets = groups.filter((pane) => pane.id !== sourcePaneId);
  const title = mode === "group"
    ? `合并分组 · ${source?.views.length ?? 0} 个视图`
    : `移动视图 · ${sourceView?.title || sourceSession?.profile.name || "会话不可用"}`;
  return (
    <div className="dialog-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <div className="wind-dialog workspace-group-move-dialog">
        <header className="dialog-title">
          <span>{title}</span>
          <button type="button" title="关闭" aria-label="关闭" onClick={onClose}><X size={18} /></button>
        </header>
        <div className="workspace-group-targets">
          {targets.map((pane) => {
            const index = groups.findIndex((group) => group.id === pane.id);
            const activeView = workspacePaneActiveView(pane);
            const activeSession = sessions.find((session) => session.profile.id === activeView.sessionId);
            const mergedCount = mode === "group"
              ? pane.views.length + (source?.views.length ?? 0)
              : pane.views.length + 1;
            const isFull = mergedCount > MAX_WORKSPACE_GROUP_TABS;
            return (
              <button
                type="button"
                key={pane.id}
                disabled={isFull}
                aria-label={`${mode === "group" ? "合并分组到" : "移动视图到"}分组 ${index + 1}`}
                onClick={() => onMove(sourcePaneId, pane.id)}
              >
                <span className="workspace-group-index">{index + 1}</span>
                <strong>{activeView.title || activeSession?.profile.name || "会话不可用"}</strong>
                <small>{pane.views.length} 个视图{isFull ? " · 超出上限" : mode === "group" ? ` · 合并后 ${mergedCount}` : ""}</small>
                <ArrowRightLeft size={14} />
              </button>
            );
          })}
          {!targets.length ? <div className="empty-pane top">当前没有其他可用分组</div> : null}
        </div>
      </div>
    </div>
  );
}

function TerminalPaneGrid({
  root,
  sessions,
  activePaneId,
  zoomedPaneId,
  activeId,
  eventsBySession,
  oneKeys,
  oneKeyCompletionEnabled,
  completionSettings,
  completionHistory,
  completionQuickCommands,
  mouseReporting,
  copyOnSelect,
  blockSelection,
  onInput,
  onOneKeyCompletion,
  onKeyModeChange,
  onConnect,
  onDisconnect,
  onSetSerialLine,
  onSendSerialBreak,
  onActivate,
  onCloseView,
  onRenameView,
  onOpenViewContextMenu,
  onMoveViewDrop,
  onSplitViewDrop,
  onSplitRatioChange,
}: {
  root: WorkspaceNode | null;
  sessions: SessionSummary[];
  activePaneId: string;
  zoomedPaneId: string;
  activeId: string;
  eventsBySession: Record<string, SessionEvent[]>;
  oneKeys: readonly OneKeySummary[];
  oneKeyCompletionEnabled: boolean;
  completionSettings: unknown;
  completionHistory: readonly string[];
  completionQuickCommands: readonly QuickCommand[];
  mouseReporting: boolean;
  copyOnSelect: boolean;
  blockSelection: boolean;
  onInput: (sessionId: string, text: string, origin: SyncInputOrigin) => void;
  onOneKeyCompletion: (
    sessionId: string,
    oneKeyId: string,
    field: OneKeyPromptField,
    promptEventId: string,
  ) => Promise<void>;
  onKeyModeChange: (paneId: string, viewId: string, keyMode: TerminalKeyMode) => void;
  onConnect: (sessionId: string) => void;
  onDisconnect: (sessionId: string) => void;
  onSetSerialLine: (sessionId: string, line: "dtr" | "rts", value: boolean) => void;
  onSendSerialBreak: (sessionId: string) => void;
  onActivate: (paneId: string, viewId: string) => void;
  onCloseView: (paneId: string, viewIds: string[]) => void;
  onRenameView: (paneId: string, viewId?: string) => void;
  onOpenViewContextMenu: (paneId: string, viewId: string, x: number, y: number) => void;
  onMoveViewDrop: (sourcePaneId: string, viewId: string, targetPaneId: string, targetIndex: number) => void;
  onSplitViewDrop: (
    sourcePaneId: string,
    viewId: string,
    targetPaneId: string,
    edge: WorkspacePaneDirection,
  ) => void;
  onSplitRatioChange: (splitId: string, ratio: number) => void;
}) {
  if (!root) {
    const active = sessions.find((session) => session.profile.id === activeId);
    return <TerminalCanvas active={active} events={active ? eventsBySession[active.profile.id] ?? [] : []} focused oneKeys={oneKeys} oneKeyCompletionEnabled={oneKeyCompletionEnabled} completionSettings={completionSettings} completionHistory={completionHistory} completionQuickCommands={completionQuickCommands} mouseReporting={mouseReporting} copyOnSelect={copyOnSelect} blockSelection={blockSelection} onInput={onInput} onOneKeyCompletion={onOneKeyCompletion} />;
  }

  return (
    <div className={`terminal-pane-grid recursive ${blockSelection ? "block-selection" : ""}`}>
      <TerminalWorkspaceNode
        node={root}
        sessions={sessions}
        activePaneId={activePaneId}
        zoomedPaneId={zoomedPaneId}
        canMoveView={workspacePaneLeaves(root).length > 1}
        canCloseView={workspacePaneLeaves(root).reduce((count, pane) => count + pane.views.length, 0) > 1}
        eventsBySession={eventsBySession}
        oneKeys={oneKeys}
        oneKeyCompletionEnabled={oneKeyCompletionEnabled}
        completionSettings={completionSettings}
        completionHistory={completionHistory}
        completionQuickCommands={completionQuickCommands}
        mouseReporting={mouseReporting}
        copyOnSelect={copyOnSelect}
        blockSelection={blockSelection}
        onInput={onInput}
        onOneKeyCompletion={onOneKeyCompletion}
        onKeyModeChange={onKeyModeChange}
        onConnect={onConnect}
        onDisconnect={onDisconnect}
        onSetSerialLine={onSetSerialLine}
        onSendSerialBreak={onSendSerialBreak}
        onActivate={onActivate}
        onCloseView={onCloseView}
        onRenameView={onRenameView}
        onOpenViewContextMenu={onOpenViewContextMenu}
        onMoveViewDrop={onMoveViewDrop}
        onSplitViewDrop={onSplitViewDrop}
        onSplitRatioChange={onSplitRatioChange}
      />
    </div>
  );
}

type TerminalWorkspaceNodeProps = {
  node: WorkspaceNode;
  sessions: SessionSummary[];
  activePaneId: string;
  zoomedPaneId: string;
  canMoveView: boolean;
  canCloseView: boolean;
  eventsBySession: Record<string, SessionEvent[]>;
  oneKeys: readonly OneKeySummary[];
  oneKeyCompletionEnabled: boolean;
  completionSettings: unknown;
  completionHistory: readonly string[];
  completionQuickCommands: readonly QuickCommand[];
  mouseReporting: boolean;
  copyOnSelect: boolean;
  blockSelection: boolean;
  onInput: (sessionId: string, text: string, origin: SyncInputOrigin) => void;
  onOneKeyCompletion: (
    sessionId: string,
    oneKeyId: string,
    field: OneKeyPromptField,
    promptEventId: string,
  ) => Promise<void>;
  onKeyModeChange: (paneId: string, viewId: string, keyMode: TerminalKeyMode) => void;
  onConnect: (sessionId: string) => void;
  onDisconnect: (sessionId: string) => void;
  onSetSerialLine: (sessionId: string, line: "dtr" | "rts", value: boolean) => void;
  onSendSerialBreak: (sessionId: string) => void;
  onActivate: (paneId: string, viewId: string) => void;
  onCloseView: (paneId: string, viewIds: string[]) => void;
  onRenameView: (paneId: string, viewId?: string) => void;
  onOpenViewContextMenu: (paneId: string, viewId: string, x: number, y: number) => void;
  onMoveViewDrop: (sourcePaneId: string, viewId: string, targetPaneId: string, targetIndex: number) => void;
  onSplitViewDrop: (
    sourcePaneId: string,
    viewId: string,
    targetPaneId: string,
    edge: WorkspacePaneDirection,
  ) => void;
  onSplitRatioChange: (splitId: string, ratio: number) => void;
};

function TerminalWorkspaceNode(props: TerminalWorkspaceNodeProps) {
  const { node } = props;
  if (node.kind === "split") return <TerminalSplitNode {...props} node={node} />;
  const activeView = workspacePaneActiveView(node);
  const session = props.sessions.find((item) => item.profile.id === activeView.sessionId);
  const serialConnection = session?.profile.connection.kind === "serial" ? session.profile.connection : null;
  const connectionAction = session ? sessionConnectionAction(session.runtime.status) : null;
  const groupViews = node.views.map((view) => ({
    view,
    session: props.sessions.find((item) => item.profile.id === view.sessionId),
  })).filter((item): item is { view: WorkspaceView; session: SessionSummary } => Boolean(item.session));
  return (
    <section
      className={node.id === props.activePaneId ? "terminal-pane active" : "terminal-pane"}
      data-pane-id={node.id}
      onMouseDown={() => props.onActivate(node.id, node.activeViewId)}
      onDragOver={(event) => {
        if (!isWorkspaceViewDrag(event.dataTransfer)) return;
        const target = event.target instanceof Element ? event.target : null;
        if (target?.closest(".workspace-pane-tabs")) return;
        event.preventDefault();
        event.stopPropagation();
        event.dataTransfer.dropEffect = "move";
        clearWorkspaceDropIndicators();
        event.currentTarget.dataset.viewDropZone = workspaceViewDropZone(
          event.currentTarget,
          event.clientX,
          event.clientY,
        );
      }}
      onDragLeave={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
          delete event.currentTarget.dataset.viewDropZone;
        }
      }}
      onDrop={(event) => {
        const target = event.target instanceof Element ? event.target : null;
        if (target?.closest(".workspace-pane-tabs")) return;
        const source = readWorkspaceViewDrag(event.dataTransfer);
        const zone = readWorkspaceViewDropZone(event.currentTarget.dataset.viewDropZone);
        clearWorkspaceDropIndicators();
        if (!source || !zone) return;
        event.preventDefault();
        event.stopPropagation();
        if (zone === "center") {
          props.onMoveViewDrop(source.paneId, source.viewId, node.id, node.views.length);
        } else {
          props.onSplitViewDrop(source.paneId, source.viewId, node.id, zone);
        }
      }}
    >
      <header>
        <div
          className="workspace-pane-tabs"
          role="tablist"
          aria-label="分组视图"
          onDragOver={(event) => {
            if (!isWorkspaceViewDrag(event.dataTransfer) || event.target !== event.currentTarget) return;
            event.preventDefault();
            event.dataTransfer.dropEffect = "move";
            markWorkspaceDropTarget(event.currentTarget, "end");
          }}
          onDragLeave={(event) => {
            if (!event.currentTarget.contains(event.relatedTarget as Node | null)) clearWorkspaceDropTarget(event.currentTarget);
          }}
          onDrop={(event) => {
            if (event.target !== event.currentTarget) return;
            const source = readWorkspaceViewDrag(event.dataTransfer);
            clearWorkspaceDropIndicators();
            if (!source) return;
            event.preventDefault();
            event.stopPropagation();
            props.onMoveViewDrop(source.paneId, source.viewId, node.id, node.views.length);
          }}
        >
          {groupViews.map(({ view, session: item }, index) => {
            const isActiveView = view.id === node.activeViewId;
            const label = view.title || item.profile.name;
            return (
              <div
                className={`workspace-pane-tab status-${item.runtime.status}${isActiveView ? " active" : ""}${view.color ? " has-color" : ""}`}
                role="presentation"
                data-view-id={view.id}
                draggable
                key={view.id}
                style={view.color ? { "--workspace-view-color": view.color } as CSSProperties : undefined}
                onContextMenu={(event) => {
                  event.preventDefault();
                  event.stopPropagation();
                  props.onOpenViewContextMenu(node.id, view.id, event.clientX, event.clientY);
                }}
                onDragStart={(event) => {
                  event.stopPropagation();
                  event.dataTransfer.effectAllowed = "move";
                  event.dataTransfer.setData(WORKSPACE_VIEW_DRAG_TYPE, JSON.stringify({ paneId: node.id, viewId: view.id }));
                }}
                onDragEnd={clearWorkspaceDropIndicators}
                onDragOver={(event) => {
                  if (!isWorkspaceViewDrag(event.dataTransfer)) return;
                  event.preventDefault();
                  event.stopPropagation();
                  event.dataTransfer.dropEffect = "move";
                  const position = event.clientX < event.currentTarget.getBoundingClientRect().left + event.currentTarget.offsetWidth / 2
                    ? "before"
                    : "after";
                  markWorkspaceDropTarget(event.currentTarget, position);
                }}
                onDragLeave={(event) => {
                  if (!event.currentTarget.contains(event.relatedTarget as Node | null)) clearWorkspaceDropTarget(event.currentTarget);
                }}
                onDrop={(event) => {
                  const source = readWorkspaceViewDrag(event.dataTransfer);
                  const position = event.currentTarget.dataset.dropPosition;
                  clearWorkspaceDropIndicators();
                  if (!source || (position !== "before" && position !== "after")) return;
                  event.preventDefault();
                  event.stopPropagation();
                  props.onMoveViewDrop(source.paneId, source.viewId, node.id, position === "before" ? index : index + 1);
                }}
              >
                <button
                  type="button"
                  role="tab"
                  aria-selected={isActiveView}
                  className="workspace-pane-tab-label"
                  title={label}
                  onMouseDown={(event) => event.stopPropagation()}
                  onDoubleClick={(event) => {
                    event.stopPropagation();
                    props.onRenameView(node.id, view.id);
                  }}
                  onClick={(event) => {
                    event.stopPropagation();
                    props.onActivate(node.id, view.id);
                  }}
                >
                  <span className="tab-mark" />
                  <span>{label}</span>
                </button>
                <button
                  type="button"
                  className="workspace-pane-tab-close"
                  title={`关闭视图 ${label}`}
                  aria-label={`关闭视图 ${label}`}
                  disabled={!props.canCloseView}
                  onMouseDown={(event) => event.stopPropagation()}
                  onClick={(event) => {
                    event.stopPropagation();
                    props.onCloseView(node.id, [view.id]);
                  }}
                >
                  <X size={11} />
                </button>
              </div>
            );
          })}
          {!groupViews.length ? <strong>会话不可用</strong> : null}
        </div>
        {session && serialConnection && session.runtime.status === "connected" ? (
          <div className="pane-serial-tools" aria-label="串口线路控制">
            <button
              type="button"
              className={serialConnection.dtr ? "active" : ""}
              aria-pressed={serialConnection.dtr}
              title="切换 DTR"
              onClick={(event) => {
                event.stopPropagation();
                props.onSetSerialLine(session.profile.id, "dtr", !serialConnection.dtr);
              }}
            >DTR</button>
            <button
              type="button"
              className={serialConnection.rts ? "active" : ""}
              aria-pressed={serialConnection.rts}
              title="切换 RTS"
              onClick={(event) => {
                event.stopPropagation();
                props.onSetSerialLine(session.profile.id, "rts", !serialConnection.rts);
              }}
            >RTS</button>
            <button
              type="button"
              title="发送 Break"
              onClick={(event) => {
                event.stopPropagation();
                props.onSendSerialBreak(session.profile.id);
              }}
            >BRK</button>
          </div>
        ) : null}
        {session ? (
          <button
            type="button"
            className={`connection-toggle ${session.runtime.status}`}
            title={`${connectionAction === "disconnect" ? "断开" : "连接"} ${session.profile.name}`}
            aria-label={`${connectionAction === "disconnect" ? "断开" : "连接"} ${session.profile.name}`}
            onClick={(event) => {
              event.stopPropagation();
              if (connectionAction === "disconnect") props.onDisconnect(session.profile.id);
              else props.onConnect(session.profile.id);
            }}
          >
            {connectionAction === "disconnect" ? <Square size={11} /> : <Play size={12} />}
          </button>
        ) : null}
      </header>
      <TerminalCanvas
        viewId={activeView.id}
        active={session}
        events={session ? props.eventsBySession[activeView.sessionId] ?? [] : []}
        focused={node.id === props.activePaneId}
        oneKeys={props.oneKeys}
        oneKeyCompletionEnabled={props.oneKeyCompletionEnabled}
        completionSettings={props.completionSettings}
        completionHistory={props.completionHistory}
        completionQuickCommands={props.completionQuickCommands}
        mouseReporting={props.mouseReporting}
        copyOnSelect={props.copyOnSelect}
        blockSelection={props.blockSelection}
        keyMode={activeView.keyMode}
        onKeyModeChange={(keyMode) => props.onKeyModeChange(node.id, activeView.id, keyMode)}
        onInput={props.onInput}
        onOneKeyCompletion={props.onOneKeyCompletion}
      />
    </section>
  );
}

function TerminalSplitNode(props: Omit<TerminalWorkspaceNodeProps, "node"> & { node: WorkspaceSplitNode }) {
  const { node } = props;
  const containerRef = useRef<HTMLDivElement | null>(null);
  const firstTrack = `${node.ratio}fr`;
  const secondTrack = `${1 - node.ratio}fr`;
  const zoomFirst = Boolean(props.zoomedPaneId && findWorkspacePane(node.first, props.zoomedPaneId));
  const zoomSecond = Boolean(props.zoomedPaneId && findWorkspacePane(node.second, props.zoomedPaneId));
  const zoomed = zoomFirst || zoomSecond;
  const style: CSSProperties = node.direction === "horizontal"
    ? { gridTemplateRows: zoomed ? "minmax(0, 1fr)" : `minmax(0, ${firstTrack}) 5px minmax(0, ${secondTrack})` }
    : { gridTemplateColumns: zoomed ? "minmax(0, 1fr)" : `minmax(0, ${firstTrack}) 5px minmax(0, ${secondTrack})` };

  function updateFromPointer(event: ReactPointerEvent<HTMLButtonElement>) {
    const rect = containerRef.current?.getBoundingClientRect();
    if (!rect) return;
    const ratio = node.direction === "horizontal"
      ? (event.clientY - rect.top) / Math.max(1, rect.height)
      : (event.clientX - rect.left) / Math.max(1, rect.width);
    props.onSplitRatioChange(node.id, ratio);
  }

  function handleSplitterKey(event: React.KeyboardEvent<HTMLButtonElement>) {
    const decrease = node.direction === "horizontal" ? event.key === "ArrowUp" : event.key === "ArrowLeft";
    const increase = node.direction === "horizontal" ? event.key === "ArrowDown" : event.key === "ArrowRight";
    let ratio: number | null = decrease ? node.ratio - 0.05 : increase ? node.ratio + 0.05 : null;
    if (event.key === "Home") ratio = MIN_WORKSPACE_SPLIT_RATIO;
    if (event.key === "End") ratio = MAX_WORKSPACE_SPLIT_RATIO;
    if (ratio === null) return;
    event.preventDefault();
    props.onSplitRatioChange(node.id, ratio);
  }

  return (
    <div
      ref={containerRef}
      className={`terminal-split ${node.direction}`}
      data-split-id={node.id}
      data-zoom-branch={zoomFirst ? "first" : zoomSecond ? "second" : undefined}
      style={style}
    >
      <div className="terminal-split-child" hidden={zoomSecond}>
        <TerminalWorkspaceNode {...props} node={node.first} />
      </div>
      <button
        type="button"
        className="terminal-splitter"
        hidden={zoomed}
        role="separator"
        aria-label={node.direction === "horizontal" ? "调整上下窗格" : "调整左右窗格"}
        aria-orientation={node.direction === "horizontal" ? "horizontal" : "vertical"}
        aria-valuemin={Math.round(MIN_WORKSPACE_SPLIT_RATIO * 100)}
        aria-valuemax={Math.round(MAX_WORKSPACE_SPLIT_RATIO * 100)}
        aria-valuenow={Math.round(node.ratio * 100)}
        title={node.direction === "horizontal" ? "拖动调整上下窗格，双击复位" : "拖动调整左右窗格，双击复位"}
        onPointerDown={(event) => {
          event.currentTarget.setPointerCapture(event.pointerId);
          updateFromPointer(event);
        }}
        onPointerMove={(event) => {
          if (event.currentTarget.hasPointerCapture(event.pointerId)) updateFromPointer(event);
        }}
        onPointerUp={(event) => {
          if (event.currentTarget.hasPointerCapture(event.pointerId)) {
            event.currentTarget.releasePointerCapture(event.pointerId);
          }
        }}
        onPointerCancel={(event) => {
          if (event.currentTarget.hasPointerCapture(event.pointerId)) {
            event.currentTarget.releasePointerCapture(event.pointerId);
          }
        }}
        onDoubleClick={() => props.onSplitRatioChange(node.id, 0.5)}
        onKeyDown={handleSplitterKey}
      />
      <div className="terminal-split-child" hidden={zoomFirst}>
        <TerminalWorkspaceNode {...props} node={node.second} />
      </div>
    </div>
  );
}

type TerminalCanvasProps = {
  viewId?: string;
  active?: SessionSummary;
  events: SessionEvent[];
  focused?: boolean;
  oneKeys?: readonly OneKeySummary[];
  oneKeyCompletionEnabled?: boolean;
  completionSettings?: unknown;
  completionHistory?: readonly string[];
  completionQuickCommands?: readonly QuickCommand[];
  mouseReporting?: boolean;
  copyOnSelect?: boolean;
  blockSelection?: boolean;
  keyMode?: TerminalKeyMode;
  onKeyModeChange?: (keyMode: TerminalKeyMode) => void;
  onInput: (sessionId: string, text: string, origin: SyncInputOrigin) => void;
  onOneKeyCompletion?: (
    sessionId: string,
    oneKeyId: string,
    field: OneKeyPromptField,
    promptEventId: string,
  ) => Promise<void>;
};

function TerminalCanvas(props: TerminalCanvasProps) {
  return (
    <Suspense fallback={<div className="terminal-canvas"><div className="terminal-empty">正在加载终端...</div></div>}>
      <LazyTerminalCanvas {...props} />
    </Suspense>
  );
}

function isWorkspaceHotkeyTarget(target: EventTarget | null) {
  const element = target instanceof Element ? target : document.activeElement;
  if (element?.closest(".terminal-search-bar, .terminal-goto-line, .terminal-free-input, .terminal-one-key-completion")) return false;
  return Boolean(element?.closest(".terminal-host, .terminal-pane-grid"));
}

function consumeWorkspaceHotkey(event: KeyboardEvent) {
  event.preventDefault();
  event.stopPropagation();
}

function isModifierKeyEvent(event: KeyboardEvent) {
  return ["Alt", "Control", "Meta", "Shift"].includes(event.key);
}

function isPlainEscape(event: KeyboardEvent) {
  return event.code === "Escape" && !event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey;
}

function focusWorkspacePaneInput(paneId: string) {
  window.requestAnimationFrame(() => {
    const pane = [...document.querySelectorAll<HTMLElement>(".terminal-pane")]
      .find((item) => item.dataset.paneId === paneId);
    pane?.querySelector<HTMLTextAreaElement>(".xterm-helper-textarea")?.focus({ preventScroll: true });
  });
}

const WORKSPACE_VIEW_DRAG_TYPE = "application/x-portmate-workspace-view";
type WorkspaceViewDropZone = WorkspacePaneDirection | "center";
const workspaceViewDropZones: readonly WorkspaceViewDropZone[] = ["left", "right", "up", "down", "center"];

function isWorkspaceViewDrag(dataTransfer: DataTransfer) {
  return Array.from(dataTransfer.types).includes(WORKSPACE_VIEW_DRAG_TYPE);
}

function readWorkspaceViewDrag(dataTransfer: DataTransfer): { paneId: string; viewId: string } | null {
  if (!isWorkspaceViewDrag(dataTransfer)) return null;
  try {
    const source = JSON.parse(dataTransfer.getData(WORKSPACE_VIEW_DRAG_TYPE)) as Record<string, unknown>;
    const paneId = typeof source.paneId === "string" ? source.paneId : "";
    const viewId = typeof source.viewId === "string" ? source.viewId : "";
    if (!paneId || !viewId || paneId.length > 128 || viewId.length > 128 || /[\u0000-\u001f\u007f]/.test(`${paneId}${viewId}`)) return null;
    return { paneId, viewId };
  } catch {
    return null;
  }
}

function workspaceViewDropZone(element: HTMLElement, clientX: number, clientY: number): WorkspaceViewDropZone {
  const bounds = element.getBoundingClientRect();
  const x = Math.min(1, Math.max(0, (clientX - bounds.left) / Math.max(1, bounds.width)));
  const y = Math.min(1, Math.max(0, (clientY - bounds.top) / Math.max(1, bounds.height)));
  const edges: Array<[WorkspacePaneDirection, number]> = [
    ["left", x],
    ["right", 1 - x],
    ["up", y],
    ["down", 1 - y],
  ];
  const nearest = edges.sort((left, right) => left[1] - right[1])[0];
  return nearest[1] <= 0.24 ? nearest[0] : "center";
}

function readWorkspaceViewDropZone(value: string | undefined): WorkspaceViewDropZone | null {
  return workspaceViewDropZones.includes(value as WorkspaceViewDropZone)
    ? value as WorkspaceViewDropZone
    : null;
}

function markWorkspaceDropTarget(element: HTMLElement, position: "before" | "after" | "end") {
  clearWorkspaceDropIndicators();
  element.dataset.dropPosition = position;
}

function clearWorkspaceDropTarget(element: HTMLElement) {
  delete element.dataset.dropPosition;
}

function clearWorkspaceDropIndicators() {
  document.querySelectorAll<HTMLElement>("[data-drop-position]").forEach(clearWorkspaceDropTarget);
  document.querySelectorAll<HTMLElement>("[data-view-drop-zone]").forEach((element) => {
    delete element.dataset.viewDropZone;
  });
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
  const protocols = useMemo(() => transferProtocolsForProfile(session.profile), [session.profile]);
  const [protocol, setProtocol] = useState<TransferProtocol | "">(() => protocols[0] ?? "");
  const [source, setSource] = useState("");
  const [destination, setDestination] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const sessionTransfers = transfers.filter((task) => task.sessionId === session.profile.id);
  const runningTransfers = sessionTransfers.filter((task) => task.status === "running");
  const retryableTransfers = sessionTransfers.filter((task) => task.status === "failed" || task.status === "cancelled");
  const connected = session.runtime.status === "connected";

  useEffect(() => {
    if (!protocol || !protocols.includes(protocol)) {
      setProtocol(protocols[0] ?? "");
    }
  }, [protocol, protocols]);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setError("");
    if (!protocol) {
      setError("当前 Profile 未启用可用的传输协议。");
      return;
    }
    if (!connected) {
      setError("连接会话后才能开始传输。");
      return;
    }
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
            <select value={protocol} disabled={!protocols.length} onChange={(event) => setProtocol(event.target.value as TransferProtocol)}>
              {!protocols.length ? <option value="">未启用传输协议</option> : null}
              {protocols.map((option) => <option key={option} value={option}>{transferProtocolLabel(option)}</option>)}
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
          {!connected ? <div className="utility-status">当前会话未连接，只能查看和管理已有任务。</div> : null}
          {connected && !protocols.length ? <div className="utility-status">当前 Profile 未启用适用于此协议的传输方式。</div> : null}
          {error ? <div className="utility-error">{error}</div> : null}
        </section>
        <footer className="utility-actions">
          <button type="button" onClick={onClose}>取消</button>
          <button type="submit" disabled={busy || !connected || !protocol || !source.trim() || !destination.trim()}>{busy ? "执行中" : "开始"}</button>
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

function SysmonApplet({ session, onOpen }: { session: SessionSummary; onOpen: () => void }) {
  const [watching, setWatching] = useState(false);
  const [snapshot, setSnapshot] = useState<SysmonSnapshot | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const remote = isSshLikeProfile(session.profile);
  const canWatch = !remote || session.runtime.status === "connected";

  useEffect(() => {
    if (!canWatch) {
      setWatching(false);
      setSnapshot(null);
      setError("");
    }
  }, [canWatch]);

  useEffect(() => {
    if (!watching || !canWatch) return;
    let disposed = false;
    let running = false;

    async function sample() {
      if (running) return;
      running = true;
      if (!disposed) setBusy(true);
      try {
        const next = await invokeBackend<SysmonSnapshot>("refresh_sysmon", { sessionId: session.profile.id });
        if (!disposed) {
          setSnapshot(next);
          setError("");
        }
      } catch (error) {
        if (!disposed) setError(formatError(error));
      } finally {
        running = false;
        if (!disposed) setBusy(false);
      }
    }

    void sample();
    const timer = window.setInterval(() => void sample(), 10_000);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [watching, canWatch, session.profile.id]);

  function toggleWatching() {
    if (watching) {
      setWatching(false);
      setSnapshot(null);
      setError("");
      return;
    }
    setWatching(true);
  }

  const title = error
    ? `Sysmon: ${error}`
    : snapshot
      ? `CPU ${snapshot.cpuPercent.toFixed(1)}% · 内存 ${snapshot.memoryPercent.toFixed(1)}% · 负载 ${snapshot.loadAverage.map((value) => value.toFixed(2)).join(" / ")} · RX ${snapshot.rxKbps.toFixed(1)} KiB/s · TX ${snapshot.txKbps.toFixed(1)} KiB/s · 运行 ${formatSysmonUptime(snapshot.uptimeSeconds)} · ${formatDateTime(snapshot.ts)}`
      : remote && !canWatch
        ? "Sysmon: 远端会话未连接"
        : "Sysmon";

  return (
    <div className={`sysmon-applet${watching ? " active" : ""}${error ? " error" : ""}`} title={title}>
      <button
        type="button"
        className="sysmon-applet-toggle"
        aria-label={watching ? "停止 Sysmon 监控" : "启动 Sysmon 监控"}
        aria-pressed={watching}
        disabled={!canWatch}
        onClick={toggleWatching}
      >
        <Activity size={14} className={busy ? "loading" : ""} />
      </button>
      <button type="button" className="sysmon-applet-summary" aria-label="打开 Sysmon 详情" onClick={onOpen}>
        {snapshot ? (
          <>
            <span>CPU <b className={sysmonPercentLevel(snapshot.cpuPercent)}>{snapshot.cpuPercent.toFixed(1)}%</b></span>
            <span>MEM <b className={sysmonPercentLevel(snapshot.memoryPercent)}>{snapshot.memoryPercent.toFixed(1)}%</b></span>
          </>
        ) : <span>Sysmon</span>}
      </button>
    </div>
  );
}

function SysmonTrendView({
  history,
  mode,
  onModeChange,
  error,
}: {
  history: SysmonSnapshot[];
  mode: SysmonTrendMode;
  onModeChange: (mode: SysmonTrendMode) => void;
  error: string;
}) {
  const latest = history[history.length - 1];
  const first = history[0];
  const usageMode = mode === "usage";

  return (
    <section className="sysmon-trend-view">
      <header className="sysmon-trend-toolbar">
        <div className="sysmon-trend-modes" role="group" aria-label="趋势指标">
          <button type="button" className={usageMode ? "active" : ""} onClick={() => onModeChange("usage")}>利用率</button>
          <button type="button" className={!usageMode ? "active" : ""} onClick={() => onModeChange("network")}>网络</button>
        </div>
        <div className="sysmon-trend-legend">
          <span className={usageMode ? "cpu" : "rx"}>{usageMode ? "CPU" : "RX"} <b>{latest ? formatSysmonTrendValue(latest, mode, 0) : "-"}</b></span>
          <span className={usageMode ? "memory" : "tx"}>{usageMode ? "内存" : "TX"} <b>{latest ? formatSysmonTrendValue(latest, mode, 1) : "-"}</b></span>
        </div>
      </header>
      <div className="sysmon-trend-stage">
        <SysmonTrendCanvas history={history} mode={mode} />
        {!history.length ? <div className="sysmon-trend-empty">暂无历史样本</div> : null}
      </div>
      <footer className="sysmon-trend-range">
        <span>{first ? formatEventClock(first.ts) : "--:--:--"}</span>
        <b>{history.length} 个样本</b>
        <span>{latest ? formatEventClock(latest.ts) : "--:--:--"}</span>
      </footer>
      {error ? <div className="utility-error">{error}</div> : null}
    </section>
  );
}

function SysmonTrendCanvas({ history, mode }: { history: SysmonSnapshot[]; mode: SysmonTrendMode }) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const draw = () => {
      const width = Math.max(1, Math.floor(canvas.clientWidth));
      const height = Math.max(1, Math.floor(canvas.clientHeight));
      const scale = Math.min(2, window.devicePixelRatio || 1);
      canvas.width = Math.floor(width * scale);
      canvas.height = Math.floor(height * scale);
      const context = canvas.getContext("2d");
      if (!context) return;
      context.setTransform(scale, 0, 0, scale, 0, 0);
      context.clearRect(0, 0, width, height);

      const plot = { left: 44, right: 12, top: 14, bottom: 22 };
      const plotWidth = Math.max(1, width - plot.left - plot.right);
      const plotHeight = Math.max(1, height - plot.top - plot.bottom);
      const maximum = sysmonTrendMax(history, mode);
      context.font = '10px "JetBrains Mono", monospace';
      context.lineWidth = 1;
      context.textAlign = "right";
      context.textBaseline = "middle";

      for (let index = 0; index <= 4; index += 1) {
        const ratio = index / 4;
        const y = plot.top + plotHeight * ratio;
        context.strokeStyle = index === 4 ? "#344252" : "#26323f";
        context.beginPath();
        context.moveTo(plot.left, y + 0.5);
        context.lineTo(width - plot.right, y + 0.5);
        context.stroke();
        context.fillStyle = "#8da0b3";
        context.fillText(formatSysmonTrendAxis(maximum * (1 - ratio), mode), plot.left - 7, y);
      }

      if (!history.length) return;
      const timestamps = history.map((snapshot) => Date.parse(snapshot.ts));
      const start = timestamps[0];
      const span = Math.max(1, timestamps[timestamps.length - 1] - start);
      const xAt = (index: number) => history.length === 1
        ? plot.left + plotWidth / 2
        : plot.left + ((timestamps[index] - start) / span) * plotWidth;
      const colors = mode === "usage" ? ["#5eead4", "#f4b860"] : ["#68a7ff", "#e879f9"];

      for (const series of [0, 1] as const) {
        context.strokeStyle = colors[series];
        context.lineWidth = 1.6;
        context.lineJoin = "round";
        context.lineCap = "round";
        context.beginPath();
        history.forEach((snapshot, index) => {
          const value = Math.min(maximum, sysmonTrendValue(snapshot, mode, series));
          const x = xAt(index);
          const y = plot.top + plotHeight * (1 - value / maximum);
          if (index === 0) context.moveTo(x, y);
          else context.lineTo(x, y);
        });
        context.stroke();

        const lastIndex = history.length - 1;
        const lastValue = Math.min(maximum, sysmonTrendValue(history[lastIndex], mode, series));
        context.fillStyle = colors[series];
        context.beginPath();
        context.arc(xAt(lastIndex), plot.top + plotHeight * (1 - lastValue / maximum), 2.5, 0, Math.PI * 2);
        context.fill();
      }
    };

    draw();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(draw);
    observer?.observe(canvas);
    window.addEventListener("resize", draw);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", draw);
    };
  }, [history, mode]);

  return (
    <canvas
      ref={canvasRef}
      className="sysmon-trend-canvas"
      role="img"
      aria-label={`${mode === "usage" ? "CPU 和内存利用率" : "网络接收和发送速率"}趋势，${history.length} 个样本`}
    />
  );
}

function SysmonDialog({ session, onClose }: { session: SessionSummary; onClose: () => void }) {
  const [snapshot, setSnapshot] = useState<SysmonSnapshot | null>(null);
  const [history, setHistory] = useState<SysmonSnapshot[]>([]);
  const [tab, setTab] = useState<"processes" | "disks" | "network" | "trends">("processes");
  const [trendMode, setTrendMode] = useState<SysmonTrendMode>("usage");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [historyError, setHistoryError] = useState("");

  useEffect(() => {
    setSnapshot(null);
    setHistory([]);
    setHistoryError("");
    void loadSysmonHistory();
    void refreshSysmon();
  }, [session.profile.id]);

  async function loadSysmonHistory() {
    try {
      const loaded = await invokeBackend<SysmonSnapshot[]>("list_sysmon_history", {
        sessionId: session.profile.id,
        limit: 120,
      });
      setHistory((current) => normalizeSysmonHistory([...current, ...loaded], session.profile.id, 120));
      setHistoryError("");
    } catch (error) {
      setHistoryError(formatError(error));
    }
  }

  async function refreshSysmon() {
    setBusy(true);
    setError("");
    try {
      const next = await invokeBackend<SysmonSnapshot>("refresh_sysmon", { sessionId: session.profile.id });
      setSnapshot(next);
      setHistory((current) => mergeSysmonHistory(current, next, 120));
    } catch (error) {
      setError(formatError(error));
    } finally {
      setBusy(false);
    }
  }

  const processes = snapshot?.processes ?? [];
  const disks = snapshot?.disks ?? [];
  const interfaces = snapshot?.networkInterfaces ?? [];
  const loadAverage = snapshot?.loadAverage ?? [0, 0, 0];
  const memoryUsed = snapshot ? Math.max(0, snapshot.memoryTotalBytes - snapshot.memoryAvailableBytes) : 0;
  const scope = isSshLikeProfile(session.profile) ? "远端主机" : "本机";

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section className="wind-dialog sysmon-dialog">
        <header className="dialog-title">
          <span className="app-icon" />
          <div className="sysmon-title">
            <strong>Sysmon</strong>
            <small>{session.profile.name} · {scope}</small>
          </div>
          <button title="关闭 Sysmon" aria-label="关闭 Sysmon" onClick={onClose}><X size={20} /></button>
        </header>
        <div className="sysmon-content">
          <dl className="sysmon-summary">
            <div><dt>CPU</dt><dd>{snapshot ? `${snapshot.cpuPercent.toFixed(1)}%` : "-"}</dd></div>
            <div>
              <dt>内存</dt>
              <dd>{snapshot ? `${snapshot.memoryPercent.toFixed(1)}%` : "-"}</dd>
              <small>{snapshot?.memoryTotalBytes ? `${formatBytes(memoryUsed)} / ${formatBytes(snapshot.memoryTotalBytes)}` : "-"}</small>
            </div>
            <div><dt>负载</dt><dd>{snapshot ? loadAverage.map((value) => value.toFixed(2)).join(" · ") : "-"}</dd></div>
            <div><dt>接收</dt><dd>{snapshot ? `${snapshot.rxKbps.toFixed(1)} KiB/s` : "-"}</dd></div>
            <div><dt>发送</dt><dd>{snapshot ? `${snapshot.txKbps.toFixed(1)} KiB/s` : "-"}</dd></div>
            <div><dt>运行时间</dt><dd>{snapshot ? formatSysmonUptime(snapshot.uptimeSeconds) : "-"}</dd></div>
          </dl>

          <nav className="sysmon-tabs" aria-label="Sysmon 详情">
            <button className={tab === "processes" ? "active" : ""} onClick={() => setTab("processes")}>进程 <span>{processes.length}</span></button>
            <button className={tab === "disks" ? "active" : ""} onClick={() => setTab("disks")}>磁盘 <span>{disks.length}</span></button>
            <button className={tab === "network" ? "active" : ""} onClick={() => setTab("network")}>网络 <span>{interfaces.length}</span></button>
            <button className={tab === "trends" ? "active" : ""} onClick={() => setTab("trends")}>趋势 <span>{history.length}</span></button>
          </nav>

          <div className="sysmon-table-wrap">
            {tab === "trends" ? (
              <SysmonTrendView history={history} mode={trendMode} onModeChange={setTrendMode} error={historyError} />
            ) : null}
            {tab === "processes" ? (
              <table className="sysmon-table sysmon-process-table">
                <thead><tr><th>PID</th><th>进程</th><th>CPU</th><th>内存</th><th>RSS</th></tr></thead>
                <tbody>
                  {processes.map((process) => (
                    <tr key={process.pid}>
                      <td>{process.pid}</td><td title={process.name}>{process.name}</td><td>{process.cpuPercent.toFixed(1)}%</td><td>{process.memoryPercent.toFixed(1)}%</td><td>{formatBytes(process.rssBytes)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : null}
            {tab === "disks" ? (
              <table className="sysmon-table sysmon-disk-table">
                <thead><tr><th>挂载点</th><th>文件系统</th><th>使用率</th><th>可用</th><th>总计</th></tr></thead>
                <tbody>
                  {disks.map((disk) => (
                    <tr key={`${disk.filesystem}-${disk.mountPoint}`}>
                      <td title={disk.mountPoint}>{disk.mountPoint}</td>
                      <td title={disk.filesystem}>{disk.filesystem}</td>
                      <td><div className="sysmon-usage"><span style={{ width: `${Math.min(100, Math.max(0, disk.usedPercent))}%` }} /><b>{disk.usedPercent.toFixed(1)}%</b></div></td>
                      <td>{formatBytes(disk.availableBytes)}</td><td>{formatBytes(disk.totalBytes)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : null}
            {tab === "network" ? (
              <table className="sysmon-table sysmon-network-table">
                <thead><tr><th>接口</th><th>接收速率</th><th>发送速率</th><th>已接收</th><th>已发送</th></tr></thead>
                <tbody>
                  {interfaces.map((item) => (
                    <tr key={item.name}>
                      <td title={item.name}>{item.name}</td><td>{item.rxKbps.toFixed(1)} KiB/s</td><td>{item.txKbps.toFixed(1)} KiB/s</td><td>{formatBytes(item.rxBytes)}</td><td>{formatBytes(item.txBytes)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : null}
            {snapshot && ((tab === "processes" && !processes.length) || (tab === "disks" && !disks.length) || (tab === "network" && !interfaces.length)) ? (
              <div className="sysmon-empty">当前采样没有可显示的{tab === "processes" ? "进程" : tab === "disks" ? "磁盘" : "网络接口"}明细</div>
            ) : null}
            {!snapshot && !error && tab !== "trends" ? <div className="sysmon-empty loading"><LoaderCircle size={18} />正在采样</div> : null}
          </div>
          {error ? <div className="utility-error">{error}</div> : null}
        </div>
        <footer className="sysmon-actions">
          <span>{snapshot ? `采样时间 ${formatDateTime(snapshot.ts)}` : scope}</span>
          <button type="button" onClick={() => void refreshSysmon()} disabled={busy}>
            <RefreshCw size={14} className={busy ? "sysmon-refresh-icon loading" : "sysmon-refresh-icon"} />刷新
          </button>
          <button type="button" onClick={onClose}>关闭</button>
        </footer>
      </section>
    </div>
  );
}

function LogManagerDialog({
  sessions,
  activeId,
  onClose,
  onNotice,
}: {
  sessions: SessionSummary[];
  activeId: string;
  onClose: () => void;
  onNotice: (message: string) => void;
}) {
  const [shards, setShards] = useState<LogShardInfo[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const [preview, setPreview] = useState<LogShardPreview | null>(null);
  const [query, setQuery] = useState("");
  const [format, setFormat] = useState<LogShardInfo["format"] | "all">("all");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [bundleSessionId, setBundleSessionId] = useState(activeId || sessions[0]?.profile.id || "");
  const [bundleRedacted, setBundleRedacted] = useState(true);
  const [bundleRawLogs, setBundleRawLogs] = useState(false);
  const [bundleAttachments, setBundleAttachments] = useState(false);
  const [bundleBusy, setBundleBusy] = useState(false);
  const [bundleResult, setBundleResult] = useState<ExportSessionBundleArchiveResult | null>(null);
  const [contentQuery, setContentQuery] = useState("");
  const [searchBusy, setSearchBusy] = useState(false);
  const [searchResult, setSearchResult] = useState<SearchLogShardsResult | null>(null);
  const [activeSearchMatch, setActiveSearchMatch] = useState<LogShardSearchMatch | null>(null);
  const [archiveBusy, setArchiveBusy] = useState(false);
  const [archiveResult, setArchiveResult] = useState<ArchiveLogShardsResult | null>(null);

  const filtered = filterLogShards(shards, query, format);
  const selectedPaths = new Set(selected);
  const totalBytes = shards.reduce((sum, shard) => sum + shard.size, 0);
  const bundleAttachmentSelection = summarizeBundleAttachmentSelection(shards, selected);

  async function refreshShards() {
    setBusy(true);
    setError("");
    try {
      const next = await invokeBackend<LogShardInfo[]>("list_log_shards", {});
      setShards(next);
      const paths = new Set(next.map((shard) => shard.path));
      setSelected((current) => current.filter((path) => paths.has(path)));
      setPreview((current) => current && paths.has(current.path) ? current : null);
      setSearchResult(null);
      setActiveSearchMatch(null);
    } catch (error) {
      setError(formatError(error));
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    void refreshShards();
  }, []);

  useEffect(() => {
    if (!bundleAttachmentSelection.count || !bundleAttachmentSelection.withinLimits) setBundleAttachments(false);
  }, [bundleAttachmentSelection.count, bundleAttachmentSelection.withinLimits]);

  async function openPreview(path: string) {
    setBusy(true);
    setError("");
    try {
      setActiveSearchMatch(null);
      setPreview(await invokeBackend<LogShardPreview>("read_log_shard", { path, maxBytes: 64 * 1024 }));
    } catch (error) {
      setError(formatError(error));
    } finally {
      setBusy(false);
    }
  }

  async function deleteSelected() {
    if (!selected.length) return;
    const selectedSet = new Set(selected);
    const bytes = shards.filter((shard) => selectedSet.has(shard.path)).reduce((sum, shard) => sum + shard.size, 0);
    if (!window.confirm(`删除 ${selected.length} 个日志分片（${formatBytes(bytes)}）?`)) return;
    setBusy(true);
    setError("");
    try {
      const result = await invokeBackend<DeleteLogShardsResult>("delete_log_shards", { paths: selected });
      setSelected([]);
      setPreview(null);
      setSearchResult(null);
      setActiveSearchMatch(null);
      onNotice(`已删除 ${result.deleted} 个分片，释放 ${formatBytes(result.bytesDeleted)}`);
      const next = await invokeBackend<LogShardInfo[]>("list_log_shards", {});
      setShards(next);
    } catch (error) {
      setError(formatError(error));
    } finally {
      setBusy(false);
    }
  }

  function toggleSelected(path: string) {
    setSelected((current) => current.includes(path) ? current.filter((item) => item !== path) : [...current, path]);
  }

  async function exportBundle() {
    if (!bundleSessionId) return;
    setBundleBusy(true);
    setBundleResult(null);
    setError("");
    try {
      const result = await invokeBackend<ExportSessionBundleArchiveResult>("export_session_bundle_archive", {
        request: {
          sessionId: bundleSessionId,
          redactSecrets: bundleRedacted,
          includeRawLogs: bundleRawLogs,
          attachmentPaths: bundleAttachments ? selected : [],
        },
      });
      setBundleResult(result);
      const warning = result.warnings.length ? ` · ${result.warnings.join(" · ")}` : "";
      onNotice(`会话包已导出：${result.path}${warning}`);
    } catch (error) {
      setError(formatError(error));
    } finally {
      setBundleBusy(false);
    }
  }

  async function archiveSelected() {
    if (!selected.length) return;
    setArchiveBusy(true);
    setArchiveResult(null);
    setError("");
    try {
      const result = await invokeBackend<ArchiveLogShardsResult>("archive_log_shards", {
        request: { paths: selected },
      });
      setArchiveResult(result);
      onNotice(`已归档 ${result.shards} 个日志分片：${result.path}`);
    } catch (error) {
      setError(formatError(error));
    } finally {
      setArchiveBusy(false);
    }
  }

  async function searchShardContent() {
    if (!contentQuery.trim()) return;
    setSearchBusy(true);
    setError("");
    try {
      const result = await invokeBackend<SearchLogShardsResult>("search_log_shards", {
        request: { query: contentQuery, paths: selected, limit: 200 },
      });
      setSearchResult(result);
      setActiveSearchMatch(result.matches[0] ?? null);
      setPreview(null);
    } catch (error) {
      setError(formatError(error));
    } finally {
      setSearchBusy(false);
    }
  }

  function clearContentSearch() {
    setSearchResult(null);
    setActiveSearchMatch(null);
  }

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section className="wind-dialog log-manager-dialog">
        <header className="dialog-title">
          <FileText size={17} />
          <strong>日志管理</strong>
          <button type="button" onClick={onClose}><X size={20} /></button>
        </header>
        <div className="log-manager-content">
          <div className="log-manager-toolbar">
            <strong>{shards.length} 个分片 · {formatBytes(totalBytes)}</strong>
            <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="筛选路径" />
            <select value={format} onChange={(event) => setFormat(event.target.value as LogShardInfo["format"] | "all")} aria-label="日志格式">
              <option value="all">全部格式</option>
              <option value="raw">Raw</option>
              <option value="txt">Text</option>
              <option value="jsonl">JSONL</option>
            </select>
            <div className="log-manager-toolbar-actions">
              <button type="button" title="刷新日志分片" aria-label="刷新日志分片" onClick={() => void refreshShards()} disabled={busy}><RefreshCw size={15} /></button>
              <button type="button" title="归档选中分片" aria-label="归档选中分片" onClick={() => void archiveSelected()} disabled={archiveBusy || !selected.length}><Archive size={15} /></button>
              <button className="danger" type="button" title="删除选中分片" aria-label="删除选中分片" onClick={() => void deleteSelected()} disabled={busy || !selected.length}><Trash2 size={15} /></button>
            </div>
          </div>
          <div className="log-manager-selection">
            <span>{searchResult ? `${searchResult.matches.length} 条命中 · ${searchResult.filesScanned} 文件 · ${formatBytes(searchResult.bytesScanned)}${searchResult.truncated ? " · 已截断" : ""}` : `${filtered.length} 项 · 已选 ${selected.length}`}</span>
            {!searchResult ? <button type="button" onClick={() => setSelected(selectVisibleLogShards(selected, filtered))} disabled={!filtered.length}>全选结果</button> : null}
            {!searchResult ? <button type="button" onClick={() => setSelected([])} disabled={!selected.length}>清除</button> : null}
          </div>
          <div className="log-content-search">
            <Search size={14} />
            <input value={contentQuery} onChange={(event) => setContentQuery(event.target.value)} onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                void searchShardContent();
              }
            }} placeholder="搜索 Text / JSONL 内容" />
            <button type="button" onClick={() => void searchShardContent()} disabled={searchBusy || !contentQuery.trim()}>{searchBusy ? "搜索中" : selected.length ? "搜索选中" : "搜索全部"}</button>
            <button type="button" onClick={clearContentSearch} disabled={!searchResult}>返回分片</button>
          </div>
          <div className="log-bundle-panel">
            {archiveResult ? (
              <div className="log-bundle-result log-archive-result">
                <code title={archiveResult.path}>{archiveResult.path}</code>
                <span>{archiveResult.shards} 分片 · 源 {formatBytes(archiveResult.sourceBytes)} · 包 {formatBytes(archiveResult.size)} · SHA-256 {archiveResult.sha256.slice(0, 16)}...</span>
                <button type="button" title="复制归档信息" aria-label="复制归档信息" onClick={() => void navigator.clipboard?.writeText(`${archiveResult.path}\n${archiveResult.checksumPath}\nSHA-256 ${archiveResult.sha256}`).catch(() => {})}><Copy size={14} /></button>
              </div>
            ) : null}
            <div className="log-bundle-controls">
              <select value={bundleSessionId} onChange={(event) => setBundleSessionId(event.target.value)} aria-label="导出会话">
                {sessions.map((session) => <option key={session.profile.id} value={session.profile.id}>{session.profile.name}</option>)}
              </select>
              <label><input type="checkbox" checked={bundleRedacted} onChange={(event) => {
                setBundleRedacted(event.target.checked);
                if (event.target.checked) {
                  setBundleRawLogs(false);
                  setBundleAttachments(false);
                }
              }} />脱敏</label>
              <label className={bundleRedacted ? "disabled" : ""}><input type="checkbox" checked={bundleRawLogs} disabled={bundleRedacted} onChange={(event) => setBundleRawLogs(event.target.checked)} />Raw 片段</label>
              <label
                className={bundleRedacted || !bundleAttachmentSelection.count || !bundleAttachmentSelection.withinLimits ? "disabled" : ""}
                title={bundleRedacted ? "附件不会自动脱敏" : bundleAttachmentSelection.withinLimits ? "" : "附件最多 32 项，单项不超过 16 MiB，且合计不超过 32 MiB"}
              ><input
                type="checkbox"
                checked={bundleAttachments}
                disabled={bundleRedacted || !bundleAttachmentSelection.count || !bundleAttachmentSelection.withinLimits}
                onChange={(event) => setBundleAttachments(event.target.checked)}
              />附件 {bundleAttachmentSelection.count} · {formatBytes(bundleAttachmentSelection.bytes)}</label>
              <button type="button" onClick={() => void exportBundle()} disabled={bundleBusy || !bundleSessionId}><Package size={15} />{bundleBusy ? "导出中" : "导出会话包"}</button>
            </div>
            {bundleResult ? (
              <div className="log-bundle-result">
                <code title={bundleResult.path}>{bundleResult.path}</code>
                <span>{formatBytes(bundleResult.size)} · {bundleResult.files} 文件 · 附件 {bundleResult.attachments} · Raw {bundleResult.rawLogSegments} · {bundleResult.signatureAlgorithm}</span>
                <button type="button" title="复制会话包信息" aria-label="复制会话包信息" onClick={() => void navigator.clipboard?.writeText(`${bundleResult.path}\n${bundleResult.checksumPath}\n${bundleResult.signaturePath}\nSHA-256 ${bundleResult.sha256}\nEd25519 ${bundleResult.signingPublicKey}`).catch(() => {})}><Copy size={14} /></button>
              </div>
            ) : null}
          </div>
          <div className="log-manager-main">
            <div className="log-shard-list">
              {searchResult ? searchResult.matches.map((match) => (
                <button key={`${match.path}-${match.byteOffset}`} className={`log-search-result ${activeSearchMatch?.path === match.path && activeSearchMatch.byteOffset === match.byteOffset ? "active" : ""}`} type="button" onClick={() => {
                  setActiveSearchMatch(match);
                  setPreview(null);
                }}>
                  <strong>{match.path}:{match.line}</strong>
                  <span>{match.text}</span>
                </button>
              )) : filtered.map((shard) => (
                <div key={shard.path} className={`log-shard-row ${preview?.path === shard.path ? "active" : ""}`}>
                  <input type="checkbox" checked={selectedPaths.has(shard.path)} onChange={() => toggleSelected(shard.path)} aria-label={`选择 ${shard.path}`} />
                  <button type="button" onClick={() => void openPreview(shard.path)} title={shard.path}>
                    <strong>{shard.path}</strong>
                    <span>{shard.format.toUpperCase()} · {formatBytes(shard.size)}{shard.modifiedAt ? ` · ${new Date(shard.modifiedAt).toLocaleString()}` : ""}</span>
                  </button>
                </div>
              ))}
              {searchResult && !searchResult.matches.length ? <div className="empty-pane top">没有内容命中</div> : null}
              {!searchResult && !filtered.length ? <div className="empty-pane top">没有日志分片</div> : null}
            </div>
            <div className="log-preview">
              <header>
                <strong>{activeSearchMatch ? `${activeSearchMatch.path}:${activeSearchMatch.line}` : preview?.path ?? "预览"}</strong>
                {activeSearchMatch ? <span>{activeSearchMatch.format.toUpperCase()} · offset {activeSearchMatch.byteOffset}</span> : preview ? <span>{preview.encoding.toUpperCase()} · {formatBytes(preview.bytesRead)}{preview.truncated ? " · 尾部" : ""}</span> : null}
              </header>
              <pre>{activeSearchMatch?.text ?? preview?.content ?? "选择日志分片查看内容"}</pre>
            </div>
          </div>
          {searchResult?.warnings.length ? <div className="utility-status">{searchResult.warnings.join(" · ")}</div> : null}
          {error ? <div className="utility-error">{error}</div> : null}
        </div>
        <footer className="utility-actions">
          <button type="button" onClick={onClose}>关闭</button>
        </footer>
      </section>
    </div>
  );
}

function KeyManagerDialog({
  hostKeys,
  sessions,
  onChange,
  onProfileChange,
  onProfilesChange,
  onClose,
}: {
  hostKeys: HostKeyStore;
  sessions: SessionSummary[];
  onChange: (store: HostKeyStore) => void;
  onProfileChange: (summary: SessionSummary) => void;
  onProfilesChange: (summaries: SessionSummary[]) => void;
  onClose: () => void;
}) {
  const sshSessions = sessions.filter((session) => isSshLikeProfile(session.profile));
  const credentialSessions = sessions.filter((session) => (
    session.profile.connection.kind === "ssh"
    || session.profile.connection.kind === "tmux"
    || session.profile.connection.kind === "tcp"
    || session.profile.connection.kind === "telnet"
  ));
  const [profileId, setProfileId] = useState(sshSessions[0]?.profile.id ?? "");
  const [knownHostsText, setKnownHostsText] = useState("");
  const [exportText, setExportText] = useState("");
  const [agentKeys, setAgentKeys] = useState<IdentityRef[]>([]);
  const [clientKeyQuery, setClientKeyQuery] = useState("");
  const [clientKeySourceFilter, setClientKeySourceFilter] = useState<IdentityRef["source"] | "all">("all");
  const [clientKeyProfileFilter, setClientKeyProfileFilter] = useState("all");
  const [clientKeyGroupBy, setClientKeyGroupBy] = useState<ClientIdentityGroupBy>("profile");
  const [selectedClientKeyIds, setSelectedClientKeyIds] = useState<string[]>([]);
  const [editingClientKeyId, setEditingClientKeyId] = useState("");
  const [clientKeyEditDraft, setClientKeyEditDraft] = useState<ClientIdentityEditDraft | null>(null);
  const [clientKeyPrivateKey, setClientKeyPrivateKey] = useState("");
  const [clientKeyPassphrase, setClientKeyPassphrase] = useState("");
  const [clientKeyStorage, setClientKeyStorage] = useState<SecretStorageChoice>("auto");
  const [clientKeyMutationBusy, setClientKeyMutationBusy] = useState(false);
  const [selectedAgentKeyIds, setSelectedAgentKeyIds] = useState<string[]>([]);
  const [privateKeyLabel, setPrivateKeyLabel] = useState("profile key");
  const [privateKeyText, setPrivateKeyText] = useState("");
  const [privateKeyStorage, setPrivateKeyStorage] = useState<SecretStorageChoice>("auto");
  const [portableVault, setPortableVault] = useState<PortableVaultStatus | null>(null);
  const [portableVaultPassword, setPortableVaultPassword] = useState("");
  const [portableVaultCurrentPassword, setPortableVaultCurrentPassword] = useState("");
  const [portableVaultNewPassword, setPortableVaultNewPassword] = useState("");
  const [portableVaultConfirmPassword, setPortableVaultConfirmPassword] = useState("");
  const [portableVaultFeedback, setPortableVaultFeedback] = useState<{ kind: "error" | "status"; message: string } | null>(null);
  const [portableVaultBusy, setPortableVaultBusy] = useState(false);
  const [migrationTarget, setMigrationTarget] = useState<SecretStorage>("portable");
  const [migrationScopeProfileId, setMigrationScopeProfileId] = useState<"all" | string>("all");
  const [migrationCleanupSource, setMigrationCleanupSource] = useState(true);
  const [migrationBusy, setMigrationBusy] = useState<"preview" | "migrate" | null>(null);
  const [migrationPreviewState, setMigrationPreviewState] = useState<{ request: ProfileSecretMigrationRequest; preview: ProfileSecretMigrationPreview } | null>(null);
  const [migrationResult, setMigrationResult] = useState<ProfileSecretMigrationResponse | null>(null);
  const [migrationError, setMigrationError] = useState("");
  const [migrationRequiresRestart, setMigrationRequiresRestart] = useState(false);
  const [migrationRecovery, setMigrationRecovery] = useState<ProfileSecretMigrationRecoverySummary | null>(null);
  const [migrationRecoveryBusy, setMigrationRecoveryBusy] = useState(false);
  const [migrationRecoveryChecking, setMigrationRecoveryChecking] = useState(isBackendAvailable);
  const [migrationRecoveryStatusError, setMigrationRecoveryStatusError] = useState("");
  const [migrationRecoveryError, setMigrationRecoveryError] = useState("");
  const [migrationRecoveryWarnings, setMigrationRecoveryWarnings] = useState<string[]>([]);
  const [migrationDiagnosticBusy, setMigrationDiagnosticBusy] = useState(false);
  const [migrationDiagnosticResult, setMigrationDiagnosticResult] = useState<ProfileSecretMigrationDiagnosticExportResult | null>(null);
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
  const editingClientIdentityItem = clientIdentityItems.find((item) => item.selectionId === editingClientKeyId) ?? null;
  const editingClientSecretUsage = editingClientIdentityItem?.identity.secretRef
    ? clientIdentityItems.filter((item) => item.identity.secretRef === editingClientIdentityItem.identity.secretRef).length
    : 0;
  const selectedAgentKeys = agentKeys.filter((identity) => selectedAgentKeyIds.includes(identityStableKey(identity)));
  const vaultOperationBusy = portableVaultBusy || migrationBusy !== null || migrationRecoveryBusy || migrationDiagnosticBusy;
  const credentialMutationsFrozen = migrationRecoveryChecking || Boolean(migrationRecovery) || Boolean(migrationRecoveryStatusError);
  const credentialMutationControlsDisabled = vaultOperationBusy || credentialMutationsFrozen;
  const migrationControlsDisabled = credentialMutationControlsDisabled || migrationRequiresRestart;
  const migrationCleanupSummary = migrationResult ? summarizeProfileSecretCleanup(migrationResult.items) : null;

  useEffect(() => {
    void refreshAgentKeys();
    void refreshPortableVault();
    void refreshMigrationRecovery();
  }, []);

  useEffect(() => {
    if (!sshSessions.some((session) => session.profile.id === profileId)) {
      setProfileId(sshSessions[0]?.profile.id ?? "");
    }
    if (migrationScopeProfileId !== "all" && !credentialSessions.some((session) => session.profile.id === migrationScopeProfileId)) {
      setMigrationScopeProfileId("all");
      setMigrationPreviewState(null);
    }
  }, [profileId, sessions, migrationScopeProfileId]);

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
    if (editingClientKeyId && !validClientIds.has(editingClientKeyId)) {
      setEditingClientKeyId("");
      setClientKeyEditDraft(null);
      setClientKeyPrivateKey("");
      setClientKeyPassphrase("");
    }
  }, [sessions]);

  useEffect(() => {
    const validAgentIds = new Set(agentKeys.map(identityStableKey));
    setSelectedAgentKeyIds((current) => current.filter((id) => validAgentIds.has(id)));
  }, [agentKeys]);

  useEffect(() => {
    if (portableVault && !portableVault.unlocked) {
      clearPortableVaultRotation();
      setMigrationPreviewState(null);
    }
  }, [portableVault?.unlocked]);

  function clearPortableVaultRotation() {
    setPortableVaultCurrentPassword("");
    setPortableVaultNewPassword("");
    setPortableVaultConfirmPassword("");
  }

  function invalidateMigrationState() {
    setMigrationPreviewState(null);
    setMigrationResult(null);
    setMigrationError("");
  }

  function currentMigrationRequest(): ProfileSecretMigrationRequest {
    return buildProfileSecretMigrationRequest(
      migrationTarget,
      migrationScopeProfileId,
      credentialSessions.map((session) => session.profile.id),
      migrationCleanupSource,
    );
  }

  async function refreshAgentKeys() {
    if (!isBackendAvailable()) return;
    try {
      setAgentKeys(await invokeBackend<IdentityRef[]>("list_ssh_agent_identities", {}));
    } catch {
      setAgentKeys([]);
    }
  }

  async function refreshPortableVault() {
    if (!isBackendAvailable()) return;
    try {
      setPortableVault(await invokeBackend<PortableVaultStatus>("portable_vault_status", {}));
    } catch {
      setPortableVault(null);
    }
  }

  async function refreshMigrationRecovery(clearError = true) {
    if (!isBackendAvailable()) {
      setMigrationRecoveryChecking(false);
      return;
    }
    setMigrationRecoveryChecking(true);
    try {
      const pending = await getProfileSecretMigrationRecovery();
      setMigrationRecovery(pending);
      setMigrationRecoveryStatusError("");
      if (pending) setMigrationPreviewState(null);
      if (clearError) setMigrationRecoveryError("");
    } catch (error) {
      setMigrationRecoveryStatusError(formatError(error));
      if (clearError) setMigrationRecoveryError("");
    } finally {
      setMigrationRecoveryChecking(false);
    }
  }

  async function unlockPortableVault() {
    if (!portableVaultPassword) return;
    const existed = portableVault?.exists ?? false;
    setPortableVaultBusy(true);
    setPortableVaultFeedback(null);
    setError("");
    setStatus("");
    try {
      const next = await invokeBackend<PortableVaultStatus>("unlock_portable_vault", {
        request: { password: portableVaultPassword },
      });
      setPortableVault(next);
      setPortableVaultPassword("");
      setPortableVaultFeedback({ kind: "status", message: existed ? "Portable vault 已解锁" : "Portable vault 已创建并解锁" });
      await refreshMigrationRecovery();
    } catch (error) {
      setPortableVaultPassword("");
      setPortableVaultFeedback({ kind: "error", message: formatError(error) });
    } finally {
      setPortableVaultBusy(false);
    }
  }

  async function lockPortableVault() {
    setPortableVaultBusy(true);
    clearPortableVaultRotation();
    setPortableVaultFeedback(null);
    setError("");
    setStatus("");
    try {
      setPortableVault(await invokeBackend<PortableVaultStatus>("lock_portable_vault", {}));
      clearPortableVaultRotation();
      setPortableVaultFeedback({ kind: "status", message: "Portable vault 已锁定" });
      await refreshMigrationRecovery();
    } catch (error) {
      setPortableVaultFeedback({ kind: "error", message: formatError(error) });
    } finally {
      setPortableVaultBusy(false);
    }
  }

  async function rotatePortableVaultPassword() {
    setPortableVaultFeedback(null);
    setError("");
    setStatus("");
    if (!portableVaultCurrentPassword || !portableVaultNewPassword || !portableVaultConfirmPassword) {
      setPortableVaultFeedback({ kind: "error", message: "请填写当前密码、新密码和确认密码" });
      return;
    }
    if (Array.from(portableVaultNewPassword).length < 8) {
      setPortableVaultFeedback({ kind: "error", message: "Portable vault 新主密码至少需要 8 个字符" });
      return;
    }
    if (portableVaultNewPassword !== portableVaultConfirmPassword) {
      setPortableVaultFeedback({ kind: "error", message: "Portable vault 两次输入的新主密码不一致" });
      return;
    }
    if (portableVaultCurrentPassword === portableVaultNewPassword) {
      setPortableVaultFeedback({ kind: "error", message: "Portable vault 新主密码必须与当前密码不同" });
      return;
    }
    setPortableVaultBusy(true);
    try {
      const next = await invokeBackend<PortableVaultStatus>("rotate_portable_vault_password", {
        request: {
          currentPassword: portableVaultCurrentPassword,
          newPassword: portableVaultNewPassword,
        },
      });
      setPortableVault(next);
      clearPortableVaultRotation();
      setPortableVaultFeedback({ kind: "status", message: "Portable vault 主密码已更换" });
    } catch (error) {
      clearPortableVaultRotation();
      setPortableVaultFeedback({ kind: "error", message: formatError(error) });
    } finally {
      setPortableVaultBusy(false);
    }
  }

  async function previewProfileSecretMigration() {
    if (!portableVault?.unlocked || migrationRequiresRestart || migrationRecovery || !isBackendAvailable()) return;
    setMigrationBusy("preview");
    setMigrationError("");
    setMigrationResult(null);
    try {
      const request = currentMigrationRequest();
      const preview = await invokeBackend<ProfileSecretMigrationPreview>("preview_profile_secret_migration", { request });
      setMigrationPreviewState({ request, preview });
      setMigrationRequiresRestart(false);
    } catch (error) {
      const message = formatError(error);
      setMigrationPreviewState(null);
      setMigrationRequiresRestart(isProfileSecretMigrationRestartRequired(message));
      setMigrationError(profileSecretMigrationErrorMessage(message));
    } finally {
      setMigrationBusy(null);
    }
  }

  async function migrateProfileSecrets() {
    if (!portableVault?.unlocked || migrationRequiresRestart || migrationRecovery || !migrationPreviewState || !isBackendAvailable()) return;
    let request: ProfileSecretMigrationRequest;
    try {
      request = currentMigrationRequest();
    } catch (error) {
      setMigrationError(formatError(error));
      return;
    }
    if (!sameProfileSecretMigrationRequest(request, migrationPreviewState.request)) {
      setMigrationPreviewState(null);
      setMigrationError("迁移设置已变化，请重新预检");
      return;
    }
    if (!canExecuteProfileSecretMigration(migrationPreviewState.preview, true, false, Boolean(migrationRecovery))) return;
    setMigrationBusy("migrate");
    setMigrationError("");
    try {
      const result = await invokeBackend<ProfileSecretMigrationResponse>("migrate_profile_secrets", {
        request,
        expectedPlanToken: migrationPreviewState.preview.planToken,
      });
      onProfilesChange(result.summaries);
      setMigrationPreviewState(null);
      setMigrationResult(result);
      setMigrationRequiresRestart(false);
      setEditingClientKeyId("");
      setClientKeyEditDraft(null);
      setClientKeyPrivateKey("");
      setClientKeyPassphrase("");
      if (result.portableVaultRequiresReunlock) {
        try {
          setPortableVault(await invokeBackend<PortableVaultStatus>("lock_portable_vault", {}));
          setPortableVaultFeedback({ kind: "status", message: `已迁移 ${result.migratedSecretCount} 个 Secret；请重新解锁 Stronghold` });
        } catch (lockError) {
          setPortableVaultFeedback({ kind: "error", message: `凭据迁移已提交，但 Stronghold 自动锁定失败: ${formatError(lockError)}` });
        }
      }
    } catch (error) {
      const message = formatError(error);
      setMigrationPreviewState(null);
      setMigrationRequiresRestart(isProfileSecretMigrationRestartRequired(message));
      setMigrationError(profileSecretMigrationErrorMessage(message));
    } finally {
      await refreshMigrationRecovery();
      setMigrationBusy(null);
    }
  }

  async function recoverPendingProfileSecretMigration() {
    if (!migrationRecovery || migrationRecoveryChecking || migrationRecoveryStatusError || migrationRequiresRestart || !isBackendAvailable()) return;
    if (!canRecoverProfileSecretMigration(migrationRecovery, portableVault?.unlocked ?? false, vaultOperationBusy)) return;
    setMigrationRecoveryBusy(true);
    setMigrationRecoveryError("");
    setMigrationRecoveryWarnings([]);
    try {
      const result = await recoverProfileSecretMigration(migrationRecovery.migrationId);
      setMigrationRecovery(result.pending);
      setMigrationRecoveryWarnings(
        result.warnings.length || !result.resolved
          ? result.warnings
          : ["恢复记录已核对并清除"],
      );
      setMigrationRequiresRestart(false);
      setMigrationPreviewState(null);
      if (result.resolved) {
        setMigrationRecoveryError("");
      }
      if (result.pending?.requiresPortableVaultUnlock && portableVault?.unlocked) {
        try {
          setPortableVault(await invokeBackend<PortableVaultStatus>("lock_portable_vault", {}));
          setPortableVaultFeedback({ kind: "status", message: "恢复 checkpoint 待核对；Stronghold 已锁定，请重新解锁" });
        } catch (lockError) {
          setPortableVaultFeedback({ kind: "error", message: `恢复记录已保留，但 Stronghold 自动锁定失败: ${formatError(lockError)}` });
        }
      }
    } catch (error) {
      const message = formatError(error);
      setMigrationRequiresRestart(isProfileSecretMigrationRestartRequired(message));
      setMigrationRecoveryError(profileSecretMigrationErrorMessage(message));
    } finally {
      await refreshMigrationRecovery(false);
      setMigrationRecoveryBusy(false);
    }
  }

  async function exportPendingProfileSecretMigrationDiagnostics() {
    if (migrationRecoveryChecking || (!migrationRecovery && !migrationRecoveryStatusError) || !isBackendAvailable()) return;
    setMigrationDiagnosticBusy(true);
    setMigrationDiagnosticResult(null);
    setMigrationRecoveryError("");
    try {
      const result = await exportProfileSecretMigrationDiagnostics();
      setMigrationDiagnosticResult(result);
      setMigrationRecoveryWarnings(result.warnings);
    } catch (error) {
      setMigrationRecoveryError(formatError(error));
    } finally {
      setMigrationDiagnosticBusy(false);
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
      const expectedProfile = sessions.find((session) => session.profile.id === profile.id)?.profile ?? null;
      const saved = await invokeBackend<SessionSummary>("save_session_profile", { profile: prepareSessionProfile(profile), expectedProfile });
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
        request: { secretRef: null, secret: privateKeyText, storage: privateKeyStorage === "auto" ? null : privateKeyStorage },
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
          expectedProfile: profile,
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
        const removableItems = selected.filter((item) => !item.jumpInUse);
        skipped += selected.filter((item) => item.jumpInUse).length;
        for (const item of removableItems) {
          const response = await invokeBackend<ClientIdentityMutationResponse>("delete_client_identity", {
            request: { profileId: profile.id, identityId: item.identity.id, deleteSecret: false },
          });
          onProfileChange(response.summary);
          removed += 1;
        }
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

  function startEditClientIdentity(item: ClientIdentityItem) {
    setEditingClientKeyId(item.selectionId);
    setClientKeyEditDraft({
      profileId: item.profileId,
      identityId: item.identity.id,
      label: item.identity.label,
      source: item.identity.source,
      fingerprintSha256: item.identity.fingerprintSha256 ?? "",
      path: item.identity.path ?? "",
      secretRef: item.identity.secretRef ?? "",
    });
    setClientKeyPrivateKey("");
    setClientKeyPassphrase("");
    setClientKeyStorage(item.identity.secretRef?.startsWith("stronghold:") ? "portable" : "auto");
    setError("");
    setStatus("");
  }

  function applyClientIdentityMutation(response: ClientIdentityMutationResponse, message: string) {
    onProfileChange(response.summary);
    if (clientKeyEditDraft) {
      const connection = response.summary.profile.connection;
      if (connection.kind === "ssh" || connection.kind === "tmux") {
        const identity = connection.identityRefs.find((item) => item.id === clientKeyEditDraft.identityId);
        if (identity) {
          setClientKeyEditDraft({
            profileId: response.summary.profile.id,
            identityId: identity.id,
            label: identity.label,
            source: identity.source,
            fingerprintSha256: identity.fingerprintSha256 ?? "",
            path: identity.path ?? "",
            secretRef: identity.secretRef ?? "",
          });
        }
      }
    }
    const suffix = response.cleanupWarning
      ? ` · ${response.cleanupWarning}`
      : response.oldSecretDeleted
        ? " · 旧 secret 已清理"
        : response.oldSecretShared
          ? " · 旧 secret 仍被共享，已保留"
          : "";
    setStatus(`${message}${suffix}`);
  }

  async function saveClientIdentity() {
    if (!clientKeyEditDraft) return;
    setClientKeyMutationBusy(true);
    setError("");
    setStatus("");
    try {
      const response = await invokeBackend<ClientIdentityMutationResponse>("update_client_identity", {
        request: {
          profileId: clientKeyEditDraft.profileId,
          identityId: clientKeyEditDraft.identityId,
          label: clientKeyEditDraft.label,
          source: clientKeyEditDraft.source,
          fingerprintSha256: clientKeyEditDraft.fingerprintSha256 || null,
          path: clientKeyEditDraft.path || null,
          secretRef: clientKeyEditDraft.secretRef || null,
        },
      });
      applyClientIdentityMutation(response, "Client identity 已更新");
    } catch (error) {
      setError(formatError(error));
    } finally {
      setClientKeyMutationBusy(false);
    }
  }

  async function rotateClientIdentity() {
    if (!clientKeyEditDraft || !clientKeyPrivateKey.trim()) return;
    setClientKeyMutationBusy(true);
    setError("");
    setStatus("");
    try {
      const response = await invokeBackend<ClientIdentityMutationResponse>("rotate_client_identity", {
        request: {
          profileId: clientKeyEditDraft.profileId,
          identityId: clientKeyEditDraft.identityId,
          privateKey: clientKeyPrivateKey,
          passphrase: clientKeyPassphrase || null,
          storage: clientKeyStorage === "auto" ? null : clientKeyStorage,
        },
      });
      applyClientIdentityMutation(response, "Vault 私钥已轮换");
      setClientKeyPrivateKey("");
      setClientKeyPassphrase("");
    } catch (error) {
      setError(formatError(error));
    } finally {
      setClientKeyMutationBusy(false);
    }
  }

  async function deleteEditedClientIdentity(deleteSecret: boolean) {
    if (!clientKeyEditDraft || editingClientIdentityItem?.jumpInUse) return;
    const action = deleteSecret ? "移除该引用并清理未共享 secret" : "移除该 identity 引用";
    if (!window.confirm(`${action}？`)) return;
    setClientKeyMutationBusy(true);
    setError("");
    setStatus("");
    try {
      const response = await invokeBackend<ClientIdentityMutationResponse>("delete_client_identity", {
        request: {
          profileId: clientKeyEditDraft.profileId,
          identityId: clientKeyEditDraft.identityId,
          deleteSecret,
        },
      });
      applyClientIdentityMutation(response, "Client identity 引用已移除");
      setEditingClientKeyId("");
      setClientKeyEditDraft(null);
      setClientKeyPrivateKey("");
      setClientKeyPassphrase("");
    } catch (error) {
      setError(formatError(error));
    } finally {
      setClientKeyMutationBusy(false);
    }
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
              <button type="button" onClick={() => void copySelectedHostKeysToProfile()} disabled={credentialMutationControlsDisabled || !selectedVisibleHostKeys.length || !selectedProfile}>复制到 Profile</button>
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
                  <button onClick={() => void copyHostKeyToProfile(key)} disabled={credentialMutationControlsDisabled || !selectedProfile}>复制到 Profile</button>
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
            <div className="portable-vault-bar" title={portableVault?.path ?? "Portable Stronghold vault"}>
              <span className={portableVault?.unlocked ? "unlocked" : ""}>{portableVault?.unlocked ? <Unlock size={14} /> : <Lock size={14} />}<strong>Stronghold</strong><small>{portableVault?.unlocked ? "Unlocked" : portableVault?.exists ? "Locked" : "Not created"}</small></span>
              <input type="password" aria-label={portableVault?.exists ? "Stronghold 主密码" : "新建 Stronghold 主密码"} value={portableVaultPassword} onChange={(event) => setPortableVaultPassword(event.target.value)} placeholder={portableVault?.exists ? "Master password" : "New master password"} disabled={vaultOperationBusy || portableVault?.unlocked} onKeyDown={(event) => { if (event.key === "Enter") void unlockPortableVault(); }} />
              {portableVault?.unlocked
                ? <button className="key-icon-button" type="button" title="锁定 portable vault" aria-label="锁定 portable vault" onClick={() => void lockPortableVault()} disabled={vaultOperationBusy}><Lock size={14} /></button>
                : <button className="key-icon-button" type="button" title="解锁 portable vault" aria-label="解锁 portable vault" onClick={() => void unlockPortableVault()} disabled={vaultOperationBusy || !portableVaultPassword}><Unlock size={14} /></button>}
            </div>
            {portableVaultFeedback ? <div className={`portable-vault-feedback ${portableVaultFeedback.kind}`} role={portableVaultFeedback.kind === "error" ? "alert" : "status"} aria-live="polite">{portableVaultFeedback.message}</div> : null}
            {portableVault?.unlocked ? (
              <details className="portable-vault-rotation" onToggle={(event) => { if (!event.currentTarget.open) { clearPortableVaultRotation(); setPortableVaultFeedback(null); } }}>
                <summary><RefreshCw size={14} /><span>更换主密码</span></summary>
                <div className="portable-vault-rotation-fields">
                  <label><span>当前主密码</span><input type="password" autoComplete="current-password" value={portableVaultCurrentPassword} onChange={(event) => setPortableVaultCurrentPassword(event.target.value)} disabled={credentialMutationControlsDisabled} /></label>
                  <label><span>新主密码</span><input type="password" autoComplete="new-password" value={portableVaultNewPassword} onChange={(event) => setPortableVaultNewPassword(event.target.value)} disabled={credentialMutationControlsDisabled} /></label>
                  <label><span>确认新主密码</span><input type="password" autoComplete="new-password" value={portableVaultConfirmPassword} onChange={(event) => setPortableVaultConfirmPassword(event.target.value)} disabled={credentialMutationControlsDisabled} onKeyDown={(event) => { if (event.key === "Enter") void rotatePortableVaultPassword(); }} /></label>
                  <button type="button" onClick={() => void rotatePortableVaultPassword()} disabled={credentialMutationControlsDisabled || !portableVaultCurrentPassword || !portableVaultNewPassword || !portableVaultConfirmPassword}><RefreshCw size={14} />更换主密码</button>
                </div>
              </details>
            ) : null}
            {migrationRecovery || migrationRecoveryStatusError || migrationRecoveryError || migrationRecoveryWarnings.length ? (
              <section className={`portable-vault-migration-recovery${migrationRecovery?.disposition === "conflict" ? " conflict" : ""}`} aria-live="polite">
                <header>
                  <span>{migrationRecovery || migrationRecoveryStatusError ? <AlertCircle size={15} /> : <CheckCircle2 size={15} />}<strong>{migrationRecovery ? "待恢复的凭据迁移" : migrationRecoveryStatusError ? "无法核对凭据迁移状态" : "凭据迁移恢复完成"}</strong></span>
                  {migrationRecovery ? <small>{migrationRecoveryDispositionLabels[migrationRecovery.disposition]}</small> : null}
                </header>
                {migrationRecovery ? (
                  <>
                    <dl>
                      <div><dt>阶段</dt><dd>{migrationRecoveryStateLabels[migrationRecovery.state]}</dd></div>
                      <div><dt>Profile</dt><dd>{migrationRecovery.profileCount}</dd></div>
                      <div><dt>Secret</dt><dd>{migrationRecovery.secretCount}</dd></div>
                    </dl>
                    <p>{migrationRecovery.message}</p>
                    {migrationRecovery.requiresPortableVaultUnlock ? <p className="portable-vault-migration-recovery-unlock"><Lock size={13} />请先锁定并重新解锁 Stronghold</p> : null}
                    {migrationRecovery.disposition === "conflict" || migrationRecovery.state === "needs-resolution"
                      ? <p className="portable-vault-migration-recovery-manual">自动恢复已停止；请人工核对 Profile 引用与两侧 provider，PortMate 不会自动改写 Profile。</p>
                      : migrationRecoveryStatusError
                        ? null
                        : <button type="button" onClick={() => void recoverPendingProfileSecretMigration()} disabled={migrationRecoveryChecking || !canRecoverProfileSecretMigration(migrationRecovery, portableVault?.unlocked ?? false, vaultOperationBusy || migrationRequiresRestart)}><RefreshCw size={14} />{migrationRecoveryBusy ? "核对中" : "核对并恢复"}</button>}
                  </>
                ) : null}
                {migrationRecovery || migrationRecoveryStatusError ? <button type="button" onClick={() => void exportPendingProfileSecretMigrationDiagnostics()} disabled={migrationRecoveryChecking || vaultOperationBusy}><FileText size={14} />{migrationDiagnosticBusy ? "导出中" : "导出诊断"}</button> : null}
                {migrationDiagnosticResult ? <p className="portable-vault-migration-diagnostic-result" title={migrationDiagnosticResult.path}>诊断已导出：{migrationDiagnosticResult.path} · {formatBytes(migrationDiagnosticResult.size)} · SHA-256 {migrationDiagnosticResult.sha256.slice(0, 16)}...</p> : null}
                {migrationDiagnosticResult ? <button type="button" onClick={() => void navigator.clipboard?.writeText(`${migrationDiagnosticResult.path}\n${migrationDiagnosticResult.checksumPath}\nSHA-256 ${migrationDiagnosticResult.sha256}`).catch(() => {})}><Copy size={14} />复制导出信息</button> : null}
                {migrationRecoveryWarnings.map((warning) => <p className="portable-vault-migration-recovery-warning" key={warning}>{warning}</p>)}
                {migrationRecoveryStatusError ? <p className="portable-vault-migration-recovery-error" role="alert">状态读取失败：{migrationRecoveryStatusError}</p> : null}
                {migrationRecoveryStatusError ? <button type="button" onClick={() => void refreshMigrationRecovery()} disabled={migrationRecoveryChecking || vaultOperationBusy}><RefreshCw size={14} />{migrationRecoveryChecking ? "读取中" : "重新读取"}</button> : null}
                {migrationRecoveryError ? <p className="portable-vault-migration-recovery-error" role="alert">{migrationRecoveryError}</p> : null}
              </section>
            ) : null}
            {portableVault?.unlocked || migrationResult || migrationError || migrationRecovery ? (
              <details className="portable-vault-migration">
                <summary><ArrowRightLeft size={14} /><span>迁移 Profile 凭据</span></summary>
                {portableVault?.unlocked && !migrationRecovery ? (
                  <>
                    <div className="portable-vault-migration-config">
                      <div className="portable-vault-migration-direction" role="group" aria-label="凭据迁移方向">
                        <button type="button" aria-pressed={migrationTarget === "portable"} onClick={() => { setMigrationTarget("portable"); invalidateMigrationState(); }} disabled={migrationControlsDisabled}>Native → Stronghold</button>
                        <button type="button" aria-pressed={migrationTarget === "native"} onClick={() => { setMigrationTarget("native"); invalidateMigrationState(); }} disabled={migrationControlsDisabled}>Stronghold → Native</button>
                      </div>
                      <label><span>Profile 范围</span><select value={migrationScopeProfileId} onChange={(event) => { setMigrationScopeProfileId(event.target.value); invalidateMigrationState(); }} disabled={migrationControlsDisabled}><option value="all">全部凭据 Profile</option>{credentialSessions.map((session) => <option key={session.profile.id} value={session.profile.id}>{session.profile.name}</option>)}</select></label>
                      <label className="portable-vault-migration-cleanup"><input type="checkbox" checked={migrationCleanupSource} onChange={(event) => { setMigrationCleanupSource(event.target.checked); invalidateMigrationState(); }} disabled={migrationControlsDisabled} /><span>清理未共享的源 Secret</span></label>
                      <button className="portable-vault-migration-preview-button" type="button" onClick={() => void previewProfileSecretMigration()} disabled={migrationControlsDisabled || !credentialSessions.length}><RefreshCw size={14} />{migrationBusy === "preview" ? "预检中" : "预检"}</button>
                    </div>
                    {migrationPreviewState ? (
                      <div className="portable-vault-migration-preview" role="status" aria-live="polite">
                        <dl>
                          <div><dt>Profile</dt><dd>{migrationPreviewState.preview.affectedProfileCount}/{migrationPreviewState.preview.selectedProfileCount}</dd></div>
                          <div><dt>引用</dt><dd>{migrationPreviewState.preview.eligibleReferenceCount}</dd></div>
                          <div><dt>Secret</dt><dd>{migrationPreviewState.preview.eligibleSecretCount}</dd></div>
                          <div><dt>共享保留</dt><dd>{migrationPreviewState.preview.retainedSharedSecretCount}</dd></div>
                        </dl>
                        {migrationPreviewState.preview.alreadyTargetReferenceCount ? <p>{migrationPreviewState.preview.alreadyTargetReferenceCount} 个引用已位于目标存储</p> : null}
                        {migrationPreviewState.preview.retainedInFlightSecretCount ? <p>{migrationPreviewState.preview.retainedInFlightSecretCount} 个源 Secret 因建连中而保留</p> : null}
                        {migrationPreviewState.preview.excludedReservedReferenceCount ? <p>{migrationPreviewState.preview.excludedReservedReferenceCount} 个 MCP token 保留引用已排除</p> : null}
                        <button type="button" onClick={() => void migrateProfileSecrets()} disabled={!canExecuteProfileSecretMigration(migrationPreviewState.preview, portableVault.unlocked, migrationControlsDisabled, Boolean(migrationRecovery))}><ArrowRightLeft size={14} />{migrationBusy === "migrate" ? "迁移中" : migrationPreviewState.preview.eligibleSecretCount ? "确认迁移" : "无需迁移"}</button>
                      </div>
                    ) : null}
                  </>
                ) : null}
                {migrationResult && migrationCleanupSummary ? (
                  <div className="portable-vault-migration-result" role="status" aria-live="polite">
                    <strong>{migrationResult.migratedProfileCount} 个 Profile · {migrationResult.migratedReferenceCount} 个引用 · {migrationResult.migratedSecretCount} 个 Secret</strong>
                    <span>源清理：{migrationCleanupSummary.deleted} 删除 · {migrationCleanupSummary["retained-shared"]} 共享保留 · {migrationCleanupSummary["retained-in-use"]} 建连保留 · {migrationCleanupSummary["retained-by-request"]} 按设置保留 · {migrationCleanupSummary.failed} 失败</span>
                    {migrationResult.warnings.map((warning) => <p key={warning}>{warning}</p>)}
                  </div>
                ) : null}
                {migrationError ? <div className="portable-vault-migration-error" role="alert">{migrationError}</div> : null}
              </details>
            ) : null}
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
                <button className="key-icon-button" type="button" title={`复制到 ${selectedProfile?.name ?? "Profile"}`} aria-label={`复制到 ${selectedProfile?.name ?? "Profile"}`} onClick={() => void copyClientIdentitiesToProfile(selectedClientIdentityItems)} disabled={credentialMutationControlsDisabled || !selectedClientIdentityItems.length || !selectedProfile}><Copy size={15} /></button>
                <button className="key-icon-button" type="button" title="在各自 Profile 中置顶" aria-label="在各自 Profile 中置顶" onClick={() => void moveSelectedClientIdentitiesFirst()} disabled={credentialMutationControlsDisabled || !selectedClientIdentityItems.length}><ArrowUp size={15} /></button>
                <button className="key-icon-button danger" type="button" title="从各自 Profile 移除引用" aria-label="从各自 Profile 移除引用" onClick={() => void removeSelectedClientIdentities()} disabled={credentialMutationControlsDisabled || !selectedClientIdentityItems.length}><Trash2 size={15} /></button>
              </div>
            </div>
            <div className="client-key-groups">
              {clientIdentityGroups.map((group) => (
                <section key={group.id} className="client-key-group">
                  <header><strong>{group.label}</strong><span>{group.items.length}</span></header>
                  {group.items.map((item) => (
                    <div key={item.selectionId} className={`client-key-row${item.jumpInUse ? " in-use" : ""}${editingClientKeyId === item.selectionId ? " editing" : ""}`}>
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
                      <button className="key-icon-button client-key-edit-button" type="button" title="编辑 client identity" aria-label={`编辑 ${item.identity.label}`} onClick={() => startEditClientIdentity(item)}><Pencil size={14} /></button>
                    </div>
                  ))}
                </section>
              ))}
              {!clientIdentityItems.length ? <div className="empty-pane top">Profile 中还没有 client identity</div> : null}
              {clientIdentityItems.length && !visibleClientIdentityItems.length ? <div className="empty-pane top">当前筛选没有 client identity</div> : null}
            </div>
            {clientKeyEditDraft && editingClientIdentityItem ? (
              <section className="client-key-inspector">
                <header>
                  <span><Pencil size={14} /><strong>Identity Inspector</strong></span>
                  <button className="key-icon-button" type="button" title="关闭检查器" aria-label="关闭 identity 检查器" onClick={() => { setEditingClientKeyId(""); setClientKeyEditDraft(null); }}><X size={14} /></button>
                </header>
                <div className="client-key-inspector-grid">
                  <label><span>Label</span><input value={clientKeyEditDraft.label} onChange={(event) => setClientKeyEditDraft({ ...clientKeyEditDraft, label: event.target.value })} /></label>
                  <label><span>Source</span><select value={clientKeyEditDraft.source} onChange={(event) => setClientKeyEditDraft({ ...clientKeyEditDraft, source: event.target.value as IdentityRef["source"] })}><option value="profile-vault">Profile Vault</option><option value="system-file">System File</option><option value="agent">SSH Agent</option><option value="public-key-only">Public Key</option></select></label>
                  <label><span>Fingerprint</span><input value={clientKeyEditDraft.fingerprintSha256} onChange={(event) => setClientKeyEditDraft({ ...clientKeyEditDraft, fingerprintSha256: event.target.value })} placeholder="SHA256:..." /></label>
                  <label><span>Path / Agent comment</span><input value={clientKeyEditDraft.path} onChange={(event) => setClientKeyEditDraft({ ...clientKeyEditDraft, path: event.target.value })} disabled={clientKeyEditDraft.source === "profile-vault"} /></label>
                  <label><span>Identity ID</span><input value={clientKeyEditDraft.identityId} readOnly /></label>
                  <label><span>Profile</span><input value={editingClientIdentityItem.profileName} readOnly /></label>
                  {clientKeyEditDraft.source === "profile-vault" ? <label><span>Rotation storage</span><select value={clientKeyStorage} onChange={(event) => setClientKeyStorage(event.target.value as SecretStorageChoice)}><option value="auto">Auto / native first</option><option value="native">Native keyring</option><option value="portable" disabled={!portableVault?.unlocked}>Portable Stronghold</option></select></label> : null}
                  {clientKeyEditDraft.source === "profile-vault" ? <label className="client-key-secret-ref"><span>Secret ref</span><input value={clientKeyEditDraft.secretRef} readOnly /></label> : null}
                </div>
                <div className="client-key-impact">
                  <span>{editingClientIdentityItem.jumpInUse ? "Jump Host 使用中" : "未被 Jump Host 使用"}</span>
                  {editingClientSecretUsage > 1 ? <span>{editingClientSecretUsage} 个 identity 共享此 secret</span> : <span>{editingClientSecretUsage ? "Secret 未共享" : "无 secret"}</span>}
                </div>
                <div className="client-key-inspector-actions">
                  <button type="button" onClick={() => void saveClientIdentity()} disabled={clientKeyMutationBusy || credentialMutationControlsDisabled}>保存字段</button>
                  <button className="danger" type="button" onClick={() => void deleteEditedClientIdentity(false)} disabled={clientKeyMutationBusy || credentialMutationControlsDisabled || editingClientIdentityItem.jumpInUse}>移除引用</button>
                  {editingClientIdentityItem.identity.secretRef ? <button className="danger" type="button" onClick={() => void deleteEditedClientIdentity(true)} disabled={clientKeyMutationBusy || credentialMutationControlsDisabled || editingClientIdentityItem.jumpInUse}>移除并清理 Secret</button> : null}
                </div>
                {clientKeyEditDraft.source === "profile-vault" ? (
                  <div className="client-key-rotation">
                    <textarea value={clientKeyPrivateKey} onChange={(event) => setClientKeyPrivateKey(event.target.value)} placeholder="新的 OpenSSH private key" />
                    <input type="password" value={clientKeyPassphrase} onChange={(event) => setClientKeyPassphrase(event.target.value)} placeholder="新私钥口令（可选）" />
                    <button type="button" onClick={() => void rotateClientIdentity()} disabled={clientKeyMutationBusy || credentialMutationControlsDisabled || !clientKeyPrivateKey.trim()}><RefreshCw size={14} />轮换 Vault 私钥</button>
                  </div>
                ) : null}
              </section>
            ) : null}
            <details className="key-import-panel">
              <summary><Plus size={14} />导入私钥到 {selectedProfile?.name ?? "Profile"}</summary>
              <input value={privateKeyLabel} onChange={(event) => setPrivateKeyLabel(event.target.value)} placeholder="Key label" />
              <input type="file" accept=".pem,.key,.txt" onChange={(event) => void readPrivateKeyFile(event.currentTarget.files?.[0] ?? null)} />
              <select value={privateKeyStorage} onChange={(event) => setPrivateKeyStorage(event.target.value as SecretStorageChoice)}><option value="auto">存储：自动（优先系统）</option><option value="native">存储：系统密钥库</option><option value="portable" disabled={!portableVault?.unlocked}>存储：Portable Stronghold</option></select>
              <textarea value={privateKeyText} onChange={(event) => setPrivateKeyText(event.target.value)} placeholder="粘贴 OpenSSH private key" />
              <button onClick={() => void importPrivateKeyToProfile()} disabled={credentialMutationControlsDisabled || !selectedProfile || !privateKeyText.trim()}>导入到 Profile</button>
            </details>
            <div className="key-agent-header agent-section-header">
              <span><strong>Agent Keys</strong><small>{agentKeys.length} visible</small></span>
              <button onClick={() => void refreshAgentKeys()}>刷新</button>
            </div>
            <div className="client-key-batch agent-key-batch">
              <span>{selectedAgentKeys.length} selected</span>
              <button type="button" onClick={() => setSelectedAgentKeyIds(agentKeys.map(identityStableKey))} disabled={!agentKeys.length}>全选</button>
              <button type="button" onClick={() => setSelectedAgentKeyIds([])} disabled={!selectedAgentKeyIds.length}>清除</button>
              <button className="key-icon-button" type="button" title={`批量添加到 ${selectedProfile?.name ?? "Profile"}`} aria-label={`批量添加到 ${selectedProfile?.name ?? "Profile"}`} onClick={() => void copyAgentIdentitiesToProfile(selectedAgentKeys)} disabled={credentialMutationControlsDisabled || !selectedAgentKeys.length || !selectedProfile}><UserPlus size={15} /></button>
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
                  <button className="key-icon-button" type="button" title={`添加到 ${selectedProfile?.name ?? "Profile"}`} aria-label={`添加 ${identity.label} 到 ${selectedProfile?.name ?? "Profile"}`} onClick={() => void copyAgentIdentityToProfile(identity)} disabled={credentialMutationControlsDisabled || !selectedProfile}><UserPlus size={15} /></button>
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
  const [oneKeyId, setOneKeyId] = useState("");
  const [savePassword, setSavePassword] = useState(false);
  const [savePassphrase, setSavePassphrase] = useState(false);
  const usernameRef = useRef<HTMLInputElement | null>(null);
  const selectedOneKey = selectedSshOneKey(request.oneKeys, oneKeyId);

  useEffect(() => {
    usernameRef.current?.focus();
    usernameRef.current?.select();
  }, []);

  function selectOneKey(nextOneKeyId: string) {
    const oneKey = selectedSshOneKey(request.oneKeys, nextOneKeyId);
    setOneKeyId(oneKey?.id ?? "");
    if (oneKey) {
      setUsername(oneKey.username);
      setPassword("");
      setPassphrase("");
      setSavePassword(false);
      setSavePassphrase(false);
    }
  }

  function submit(event: FormEvent) {
    event.preventDefault();
    const nextUsername = (selectedOneKey?.username ?? username).trim();
    if (!nextUsername) {
      usernameRef.current?.focus();
      return;
    }
    onSubmit({
      username: nextUsername,
      password: !selectedOneKey && request.needsPassword ? password : null,
      passphrase: !selectedOneKey && request.hasIdentityFiles ? passphrase : null,
      oneKeyId: selectedOneKey?.id ?? null,
      savePassword: !selectedOneKey && request.needsPassword && savePassword,
      savePassphrase: !selectedOneKey && request.hasIdentityFiles && savePassphrase,
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
            <span>OneKey</span>
            <select value={oneKeyId} onChange={(event) => selectOneKey(event.target.value)} disabled={!request.oneKeys.length}>
              <option value="">{request.oneKeys.length ? "手动输入" : "没有绑定 OneKey"}</option>
              {request.oneKeys.map((oneKey) => (
                <option key={oneKey.id} value={oneKey.id}>{oneKey.label}</option>
              ))}
            </select>
          </label>
          {selectedOneKey ? (
            <div className="credential-one-key-meta">
              <KeyRound size={14} />
              <span>
                <strong>{selectedOneKey.label}</strong>
                <small>{[
                  selectedOneKey.hasPassword ? "密码" : "",
                  selectedOneKey.hasPassphrase ? "私钥口令" : "",
                  selectedOneKey.identity ? `公钥身份 · ${selectedOneKey.identity.label}` : "",
                ].filter(Boolean).join(" / ")}</small>
              </span>
            </div>
          ) : null}
          <label className="credential-field">
            <span>用户名</span>
            <input ref={usernameRef} value={username} onChange={(event) => setUsername(event.target.value)} autoComplete="username" disabled={Boolean(selectedOneKey)} />
          </label>
          {request.needsPassword ? (
            <label className="credential-field">
              <span>{selectedOneKey ? "OneKey 密码" : request.hasSavedPassword ? "登录密码(已存)" : "登录密码"}</span>
              <input value={password} onChange={(event) => setPassword(event.target.value)} type="password" autoComplete="current-password" disabled={Boolean(selectedOneKey)} placeholder={selectedOneKey ? selectedOneKey.hasPassword ? "已安全保存" : "未保存" : ""} />
            </label>
          ) : null}
          {request.needsPassword && !selectedOneKey ? (
            <label className="credential-check">
              <input type="checkbox" checked={savePassword} onChange={(event) => setSavePassword(event.target.checked)} disabled={!password} />
              <span>保存登录密码到系统密钥库</span>
            </label>
          ) : null}
          {request.hasIdentityFiles ? (
            <label className="credential-field">
              <span>{selectedOneKey ? "OneKey 私钥口令" : request.hasSavedPassphrase ? "私钥口令(已存)" : "私钥口令"}</span>
              <input value={passphrase} onChange={(event) => setPassphrase(event.target.value)} type="password" autoComplete="off" disabled={Boolean(selectedOneKey)} placeholder={selectedOneKey ? selectedOneKey.hasPassphrase ? "已安全保存" : "未保存" : "没有可留空"} />
            </label>
          ) : null}
          {request.hasIdentityFiles && !selectedOneKey ? (
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

function TerminalSettingsDialog({
  initialPrefs,
  sessions,
  syncSettings,
  workspaceKeymap,
  onPrefsChange,
  onClearCommandHistory,
  onSyncSettingsChange,
  onWorkspaceKeymapChange,
  onClose,
}: {
  initialPrefs: TerminalPrefs;
  sessions: readonly SessionSummary[];
  syncSettings: SyncInputSettings;
  workspaceKeymap: WorkspaceKeymap;
  onPrefsChange: (prefs: TerminalPrefs) => void;
  onClearCommandHistory: () => void;
  onSyncSettingsChange: (settings: SyncInputSettings) => void;
  onWorkspaceKeymapChange: (keymap: WorkspaceKeymap) => void;
  onClose: () => void;
}) {
  const [activeItem, setActiveItem] = useState("应用");
  const [prefs, setPrefs] = useState<TerminalPrefs>(initialPrefs);
  const [syncDraft, setSyncDraft] = useState(syncSettings);
  const [workspaceKeymapDraft, setWorkspaceKeymapDraft] = useState(workspaceKeymap);
  const updatePref = <K extends keyof TerminalPrefs>(key: K, value: TerminalPrefs[K]) => setPrefs((current) => ({ ...current, [key]: value }));
  const keymapConflictCount = workspaceKeymapConflicts(workspaceKeymapDraft).length;

  function savePrefs() {
    if (keymapConflictCount) return;
    const normalizedKeymap = normalizeWorkspaceKeymap(workspaceKeymapDraft);
    const normalizedPrefs = normalizeTerminalPrefs(prefs);
    saveLocalValue("portmate.terminalPrefs", normalizedPrefs);
    saveLocalValue(WORKSPACE_KEYMAP_STORAGE_KEY, normalizedKeymap);
    onPrefsChange(normalizedPrefs);
    onSyncSettingsChange(normalizeSyncInputSettings(syncDraft));
    onWorkspaceKeymapChange(normalizedKeymap);
    onClose();
  }

  return (
    <DialogFrame title="终端设置" className="terminal-settings-dialog" onClose={onClose}>
      <nav className="settings-tabs" role="tablist" aria-label="终端设置页面">
        {terminalSettingPages.map((page) => (
          <button key={page} type="button" role="tab" aria-selected={activeItem === page} className={activeItem === page ? "active" : ""} onClick={() => setActiveItem(page)}>
              {page}
            </button>
        ))}
      </nav>
      <section className="settings-content" role="tabpanel">
        <TerminalSettingsContent
          activeItem={activeItem}
          prefs={prefs}
          sessions={sessions}
          workspaceKeymap={workspaceKeymapDraft}
          updatePref={updatePref}
          onClearCommandHistory={onClearCommandHistory}
          onWorkspaceKeymapChange={setWorkspaceKeymapDraft}
          syncSettings={syncDraft}
          onSyncSettingsChange={setSyncDraft}
        />
      </section>
      <div className="dialog-footer">
        <div className={keymapConflictCount ? "dialog-note error" : "dialog-note"}>
          {keymapConflictCount ? `${keymapConflictCount} 组快捷键冲突` : ""}
        </div>
        <div className="dialog-actions inline">
          <button onClick={savePrefs} disabled={keymapConflictCount > 0}>保存</button>
          <button onClick={onClose}>取消</button>
        </div>
      </div>
    </DialogFrame>
  );
}

function TerminalSettingsContent({
  activeItem,
  prefs,
  sessions,
  workspaceKeymap,
  updatePref,
  onClearCommandHistory,
  onWorkspaceKeymapChange,
  syncSettings,
  onSyncSettingsChange,
}: {
  activeItem: string;
  prefs: TerminalPrefs;
  sessions: readonly SessionSummary[];
  workspaceKeymap: WorkspaceKeymap;
  updatePref: <K extends keyof TerminalPrefs>(key: K, value: TerminalPrefs[K]) => void;
  onClearCommandHistory: () => void;
  onWorkspaceKeymapChange: (keymap: WorkspaceKeymap) => void;
  syncSettings: SyncInputSettings;
  onSyncSettingsChange: (settings: SyncInputSettings) => void;
}) {
  switch (activeItem) {
    case "应用":
      return (
        <SettingsSection title="启动">
          <SettingRadio label="无会话(N)" checked={prefs.startupMode === "none"} onChange={() => updatePref("startupMode", "none")} name="startup-mode" />
          <SettingRadio label="上次会话(L)" checked={prefs.startupMode === "last"} onChange={() => updatePref("startupMode", "last")} name="startup-mode" />
          <SettingRadio label="指定一个会话或一组会话(S)" checked={prefs.startupMode === "specific"} onChange={() => updatePref("startupMode", "specific")} name="startup-mode" />
          {[0, 1, 2, 3].map((index) => (
            <SettingSelect
              key={index}
              label={`会话 ${index + 1}:`}
              value={prefs.startupSessions[index] ?? ""}
              options={terminalStartupSessionOptions(sessions, prefs.startupSessions[index])}
              disabled={prefs.startupMode !== "specific"}
              onChange={(value) => {
                const next = [...prefs.startupSessions];
                next[index] = value;
                updatePref("startupSessions", next);
              }}
            />
          ))}
        </SettingsSection>
      );
    case "安全":
      return (
        <SettingsSection title="安全">
          <SettingCheck label="空闲后锁屏" checked={prefs.lockOnIdle} onChange={(value) => updatePref("lockOnIdle", value)} />
          <SettingInput
            label="锁屏超时（分钟）"
            type="number"
            value={prefs.lockScreenTimeoutMinutes}
            min={MIN_SCREEN_LOCK_TIMEOUT_MINUTES}
            max={MAX_SCREEN_LOCK_TIMEOUT_MINUTES}
            step={1}
            onChange={(value) => updatePref("lockScreenTimeoutMinutes", normalizeScreenLockTimeoutMinutes(value))}
          />
          <SettingCheck label="启动时锁屏" checked={prefs.requireMasterPassword} onChange={(value) => updatePref("requireMasterPassword", value)} />
        </SettingsSection>
      );
    case "快捷键":
      return <WorkspaceKeymapSettings keymap={workspaceKeymap} onChange={onWorkspaceKeymapChange} />;
    case "同步输入":
      return (
        <>
          <SettingsSection title="目标协议">
            {allSyncProtocols.map((protocol) => (
              <SettingCheck
                key={protocol}
                label={sessionKindLabels[protocol]}
                checked={syncSettings.protocols.includes(protocol)}
                onChange={(checked) => onSyncSettingsChange({
                  ...syncSettings,
                  protocols: checked
                    ? [...syncSettings.protocols, protocol]
                    : syncSettings.protocols.filter((item) => item !== protocol),
                })}
              />
            ))}
          </SettingsSection>
          <SettingsSection title="输入变换">
            <label className="setting-row">
              <span>换行策略:</span>
              <select value={syncSettings.newlineMode} onChange={(event) => onSyncSettingsChange({ ...syncSettings, newlineMode: event.target.value as SyncNewlineMode })}>
                <option value="protocol">按协议</option>
                <option value="preserve">保持原样</option>
                <option value="lf">LF</option>
                <option value="crlf">CRLF</option>
              </select>
            </label>
            <SettingInput label="目标间延迟(ms):" type="number" value={syncSettings.delayMs} onChange={(value) => onSyncSettingsChange({ ...syncSettings, delayMs: Math.min(5000, Math.max(0, Math.trunc(Number(value) || 0))) })} />
            <SettingInput label="批量发送前缀:" value={syncSettings.prefix} onChange={(value) => onSyncSettingsChange({ ...syncSettings, prefix: value.slice(0, 1024) })} />
            <SettingInput label="批量发送后缀:" value={syncSettings.suffix} onChange={(value) => onSyncSettingsChange({ ...syncSettings, suffix: value.slice(0, 1024) })} />
          </SettingsSection>
        </>
      );
    case "自动补全":
      return (
        <>
          <SettingsSection title="完成">
            <SettingCheck label="启用自动补全(A)" checked={prefs.completionEnabled} onChange={(value) => updatePref("completionEnabled", value)} />
            <SettingCheck label="OneKey 终端提示补全(K)" checked={prefs.oneKeyCompletionEnabled} onChange={(value) => updatePref("oneKeyCompletionEnabled", value)} />
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
            <SettingInput label="保留历史天数:(D)" type="number" min={0} max={MAX_COMMAND_HISTORY_RETENTION_DAYS} step={1} value={prefs.historyRetentionDays} onChange={(value) => updatePref("historyRetentionDays", value)} />
            <SettingInput label="历史大小:(H)" type="number" min={1} max={MAX_COMMAND_HISTORY_LIMIT} step={1} value={prefs.historyLimit} onChange={(value) => updatePref("historyLimit", value)} />
          </SettingsSection>
          <SettingsSection title="存储">
            <SettingCheck label="将命令历史保存到磁盘(S)" checked={prefs.historyEnabled} onChange={(value) => updatePref("historyEnabled", value)} />
            <SettingButtonRow label="已保存的命令历史:">
              <button className="settings-secondary-button" type="button" onClick={onClearCommandHistory}>
                清除(C)
              </button>
            </SettingButtonRow>
          </SettingsSection>
        </>
      );
    case "鼠标":
      return (
        <SettingsSection title="鼠标">
          <SettingCheck label="允许终端应用接收鼠标事件" checked={prefs.mouseReporting} onChange={(value) => updatePref("mouseReporting", value)} />
          <SettingCheck label="选择即复制" checked={prefs.mouseCopyOnSelect} onChange={(value) => updatePref("mouseCopyOnSelect", value)} />
        </SettingsSection>
      );
    default:
      return null;
  }
}

function WorkspaceKeymapSettings({
  keymap,
  onChange,
}: {
  keymap: WorkspaceKeymap;
  onChange: (keymap: WorkspaceKeymap) => void;
}) {
  const [capturing, setCapturing] = useState<WorkspaceHotkeyCommandId | null>(null);
  const [capturePrefix, setCapturePrefix] = useState<{ commandId: WorkspaceHotkeyCommandId; binding: string } | null>(null);
  const [captureError, setCaptureError] = useState<WorkspaceHotkeyCommandId | null>(null);
  const captureTimerRef = useRef<number | null>(null);
  const conflicts = workspaceKeymapConflicts(keymap);
  const labels = Object.fromEntries(workspaceHotkeyCommands.map((command) => [command.id, command.label])) as Record<WorkspaceHotkeyCommandId, string>;

  useEffect(() => () => {
    if (captureTimerRef.current !== null) window.clearTimeout(captureTimerRef.current);
  }, []);

  function updateBinding(commandId: WorkspaceHotkeyCommandId, binding: string) {
    onChange({ ...keymap, [commandId]: binding });
  }

  function stopCapture(commandId?: WorkspaceHotkeyCommandId) {
    if (captureTimerRef.current !== null) window.clearTimeout(captureTimerRef.current);
    captureTimerRef.current = null;
    setCapturing((current) => !commandId || current === commandId ? null : current);
    setCapturePrefix((current) => !commandId || current?.commandId === commandId ? null : current);
    setCaptureError((current) => !commandId || current === commandId ? null : current);
  }

  function beginCapture(commandId: WorkspaceHotkeyCommandId) {
    stopCapture();
    setCapturing(commandId);
  }

  function captureBinding(event: React.KeyboardEvent<HTMLButtonElement>, commandId: WorkspaceHotkeyCommandId) {
    if (capturing !== commandId) return;
    event.preventDefault();
    event.stopPropagation();
    if (isPlainEscape(event.nativeEvent)) {
      stopCapture(commandId);
      return;
    }
    if ((event.code === "Backspace" || event.code === "Delete") && !event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey) {
      updateBinding(commandId, "");
      stopCapture(commandId);
      return;
    }
    if (event.repeat || isModifierKeyEvent(event.nativeEvent)) return;
    const binding = workspaceKeyBindingFromEvent(event);
    if (!binding) {
      setCaptureError(commandId);
      return;
    }
    if (capturePrefix?.commandId === commandId) {
      updateBinding(commandId, `${capturePrefix.binding} ${binding}`);
      stopCapture(commandId);
      return;
    }
    updateBinding(commandId, binding);
    setCapturePrefix({ commandId, binding });
    setCaptureError(null);
    captureTimerRef.current = window.setTimeout(() => stopCapture(commandId), WORKSPACE_KEY_CHORD_TIMEOUT_MS);
  }

  return (
    <SettingsSection title="快捷键">
      <div className="workspace-keymap">
        <header className="workspace-keymap-header">
          <span>命令</span>
          <span>按键</span>
          <button
            type="button"
            title="恢复全部默认快捷键"
            aria-label="恢复全部默认快捷键"
            onClick={() => {
              onChange({ ...defaultWorkspaceKeymap });
              stopCapture();
            }}
          >
            <RotateCcw size={14} />
          </button>
        </header>
        {workspaceHotkeyCommands.map((command) => {
          const conflict = conflicts.find((item) => item.commandIds.includes(command.id));
          const conflictLabels = conflict?.commandIds.filter((id) => id !== command.id).map((id) => labels[id]).join("、");
          const invalid = captureError === command.id;
          const pendingBinding = capturePrefix?.commandId === command.id ? capturePrefix.binding : "";
          const formattedBinding = formatWorkspaceKeyBinding(keymap[command.id]);
          return (
            <div key={command.id} className={`workspace-keymap-row ${conflict ? "conflict" : ""}`}>
              <span className="workspace-keymap-command">
                <strong>{command.label}</strong>
                {conflictLabels ? <small>与 {conflictLabels}{conflict?.kind === "prefix" ? " 前缀冲突" : " 冲突"}</small> : invalid ? <small>每段需要修饰键</small> : null}
              </span>
              <button
                type="button"
                className={capturing === command.id ? "workspace-key-capture capturing" : "workspace-key-capture"}
                aria-pressed={capturing === command.id}
                title={capturing === command.id ? "录入快捷键" : formattedBinding}
                onClick={() => beginCapture(command.id)}
                onBlur={() => stopCapture(command.id)}
                onKeyDown={(event) => captureBinding(event, command.id)}
              >
                {pendingBinding ? `${formatWorkspaceKeyBinding(pendingBinding)}  →  …` : capturing === command.id ? "等待第 1 键" : formattedBinding}
              </button>
              <button
                type="button"
                className="workspace-key-disable"
                title={`禁用 ${command.label} 快捷键`}
                aria-label={`禁用 ${command.label} 快捷键`}
                disabled={!keymap[command.id]}
                onClick={() => {
                  updateBinding(command.id, "");
                  stopCapture(command.id);
                }}
              >
                <Ban size={13} />
              </button>
              <button
                type="button"
                className="workspace-key-reset"
                title={`恢复 ${command.label} 默认快捷键`}
                aria-label={`恢复 ${command.label} 默认快捷键`}
                disabled={keymap[command.id] === command.defaultBinding}
                onClick={() => {
                  updateBinding(command.id, command.defaultBinding);
                  stopCapture(command.id);
                }}
              >
                <RotateCcw size={13} />
              </button>
            </div>
          );
        })}
      </div>
    </SettingsSection>
  );
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
  onSave: (proxyPasswordUpdate: ProxyPasswordUpdate) => void;
  onSaveAndConnect: (proxyPasswordUpdate: ProxyPasswordUpdate) => void;
  onClose: () => void;
}) {
  const [activeProtocol, setActiveProtocol] = useState<ProtocolTab>(() => protocolFromKind(draft.kind));
  const [activeSection, setActiveSection] = useState(initialSection);
  const [proxyPasswordUpdate, setProxyPasswordUpdate] = useState<ProxyPasswordUpdate>(null);
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
    setProxyPasswordUpdate(null);
    onDraftChange(convertDraftProtocol(draft, tab));
  }

  return (
    <DialogFrame
      title="会话设置"
      className={`session-settings-dialog ${activeSection === "会话" ? "compact" : activeSection === "传输" ? "medium" : ""}`}
      onClose={onClose}
    >
      <div className="session-settings-nav">
        <label>
          <span>会话类型</span>
          <select aria-label="会话类型" value={activeProtocol} onChange={(event) => changeProtocol(event.target.value as ProtocolTab)}>
            {protocolTabs.map((tab) => <option key={tab} value={tab}>{tab}</option>)}
          </select>
        </label>
        <label>
          <span>配置项</span>
          <select aria-label="会话配置项" value={activeSection} onChange={(event) => setActiveSection(event.target.value)}>
            {allowedSections.map((section) => <option key={section} value={section}>{section}</option>)}
          </select>
        </label>
      </div>
      <section className="session-form">
        <SessionSettingsContent activeProtocol={activeProtocol} activeSection={activeSection} draft={draft} serialPorts={serialPorts} onDraftChange={onDraftChange} proxyPasswordUpdate={proxyPasswordUpdate} onProxyPasswordUpdateChange={setProxyPasswordUpdate} />
      </section>
      <div className="dialog-actions">
        <button onClick={() => onSave(proxyPasswordUpdate)}>保存</button>
        <button onClick={() => onSaveAndConnect(proxyPasswordUpdate)}>保存并连接</button>
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
  proxyPasswordUpdate,
  onProxyPasswordUpdateChange,
}: {
  activeProtocol: ProtocolTab;
  activeSection: string;
  draft: SessionProfile;
  serialPorts: string[];
  onDraftChange: (draft: SessionProfile) => void;
  proxyPasswordUpdate: ProxyPasswordUpdate;
  onProxyPasswordUpdateChange: (update: ProxyPasswordUpdate) => void;
}) {
  if (activeSection === "会话") {
    return <SessionCommonOverviewFields draft={draft} onDraftChange={onDraftChange} />;
  }

  if (activeSection === "终端") {
    return (
      <>
        <DialogField label="终端:(T)">
          <input value={draft.terminal.term} maxLength={MAX_TERMINAL_NAME_BYTES} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, term: event.target.value } })} />
        </DialogField>
        <DialogField label="行:(R)">
          <input type="number" min={TERMINAL_PROFILE_BOUNDS.rows.min} max={TERMINAL_PROFILE_BOUNDS.rows.max} step={1} value={draft.terminal.rows} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, rows: Number(event.target.value) } })} />
        </DialogField>
        <DialogField label="列:(C)">
          <input type="number" min={TERMINAL_PROFILE_BOUNDS.cols.min} max={TERMINAL_PROFILE_BOUNDS.cols.max} step={1} value={draft.terminal.cols} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, cols: Number(event.target.value) } })} />
        </DialogField>
        <DialogField label="滚屏:(S)">
          <input type="number" min={TERMINAL_PROFILE_BOUNDS.scrollback.min} max={TERMINAL_PROFILE_BOUNDS.scrollback.max} step={1} value={draft.terminal.scrollback} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, scrollback: Number(event.target.value) } })} />
        </DialogField>
        <DialogField label="字体:(F)">
          <input value={draft.terminal.fontFamily} maxLength={MAX_TERMINAL_FONT_FAMILY_CHARACTERS} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, fontFamily: event.target.value } })} />
        </DialogField>
        <DialogField label="字号:(Z)">
          <input type="number" min={TERMINAL_PROFILE_BOUNDS.fontSize.min} max={TERMINAL_PROFILE_BOUNDS.fontSize.max} step={1} value={draft.terminal.fontSize} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, fontSize: Number(event.target.value) } })} />
        </DialogField>
        <DialogField label="主题:(M)">
          <select value={normalizeTerminalTheme(draft.terminal.theme)} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, theme: event.target.value } })}>
            {TERMINAL_THEME_OPTIONS.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
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
        <DialogField label="Raw（不脱敏）:(R)">
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
        <DialogField label="敏感字段:(S)">
          <select value={draft.logging.redactSecrets ? "redact" : "plain"} onChange={(event) => onDraftChange({ ...draft, logging: { ...draft.logging, redactSecrets: event.target.value === "redact" } })}>
            <option value="redact">隐藏</option>
            <option value="plain">完整记录</option>
          </select>
        </DialogField>
        <DialogField label="路径:(P)">
          <input value={draft.logging.pathTemplate} onChange={(event) => onDraftChange({ ...draft, logging: { ...draft.logging, pathTemplate: event.target.value } })} />
        </DialogField>
        <DialogField label="保留天数:(D)">
          <input type="number" min={0} max={3650} value={draft.logging.retentionDays ?? 0} onChange={(event) => onDraftChange({ ...draft, logging: { ...draft.logging, retentionDays: Math.min(3650, Math.max(0, Math.trunc(Number(event.target.value) || 0))) } })} />
        </DialogField>
      </>
    );
  }

  if (activeSection === "触发器") {
    return <TriggerFields draft={draft} onDraftChange={onDraftChange} />;
  }

  if (activeSection === "传输") {
    return <SessionTransferFields activeProtocol={activeProtocol} draft={draft} onDraftChange={onDraftChange} />;
  }

  if (activeProtocol === "Shell" && activeSection === "Shell") {
    return <ShellProcessFields draft={draft} onDraftChange={onDraftChange} />;
  }

  if ((activeProtocol === "SSH" || activeProtocol === "Tmux") && (activeSection === "SSH" || activeSection === "Tmux")) {
    return <SshAdvancedFields section="连接" draft={draft} onDraftChange={onDraftChange} proxyPasswordUpdate={proxyPasswordUpdate} onProxyPasswordUpdateChange={onProxyPasswordUpdateChange} />;
  }

  if ((activeProtocol === "SSH" || activeProtocol === "Tmux") && ["代理", "验证", "代理人", "密码", "公钥"].includes(activeSection)) {
    return <SshAdvancedFields section={activeSection} draft={draft} onDraftChange={onDraftChange} proxyPasswordUpdate={proxyPasswordUpdate} onProxyPasswordUpdateChange={onProxyPasswordUpdateChange} />;
  }

  if (activeProtocol === "Telnet" && activeSection === "Telnet") {
    return <TcpLikeAdvancedFields protocol="Telnet" section="连接" draft={draft} onDraftChange={onDraftChange} proxyPasswordUpdate={proxyPasswordUpdate} onProxyPasswordUpdateChange={onProxyPasswordUpdateChange} />;
  }

  if (activeProtocol === "Tcp" && activeSection === "Tcp") {
    return <TcpLikeAdvancedFields protocol="Tcp" section="连接" draft={draft} onDraftChange={onDraftChange} proxyPasswordUpdate={proxyPasswordUpdate} onProxyPasswordUpdateChange={onProxyPasswordUpdateChange} />;
  }

  if ((activeProtocol === "Telnet" || activeProtocol === "Tcp") && activeSection === "代理") {
    return <TcpLikeAdvancedFields protocol={activeProtocol} section="代理" draft={draft} onDraftChange={onDraftChange} proxyPasswordUpdate={proxyPasswordUpdate} onProxyPasswordUpdateChange={onProxyPasswordUpdateChange} />;
  }

  if (activeProtocol === "Serial" && activeSection === "串口") {
    return <SerialAdvancedFields draft={draft} serialPorts={serialPorts} onDraftChange={onDraftChange} />;
  }

  return null;
}

function SessionCommonOverviewFields({
  draft,
  onDraftChange,
}: {
  draft: SessionProfile;
  onDraftChange: (draft: SessionProfile) => void;
}) {
  const [tagsText, setTagsText] = useState(() => draft.tags.join(", "));
  useEffect(() => {
    setTagsText(draft.tags.join(", "));
  }, [draft.id]);
  return (
    <>
      <DialogField label="名称:(N)">
        <input value={draft.name} onChange={(event) => onDraftChange({ ...draft, name: normalizeSessionMetadataText(event.target.value, MAX_SESSION_PROFILE_NAME_CHARACTERS) })} />
      </DialogField>
      <DialogField label="分组:(G)">
        <input value={draft.group} onChange={(event) => onDraftChange({ ...draft, group: normalizeSessionMetadataText(event.target.value, MAX_SESSION_PROFILE_GROUP_CHARACTERS) })} placeholder="[嵌套组] a>b>c" />
      </DialogField>
      <DialogField label="标签:(L)">
        <input value={tagsText} onChange={(event) => {
          const nextText = normalizeSessionMetadataText(event.target.value, MAX_SESSION_PROFILE_TAG_INPUT_CHARACTERS);
          setTagsText(nextText);
          onDraftChange({ ...draft, tags: nextText.split(",").map((item) => item.trim()).filter(Boolean) });
        }} />
      </DialogField>
    </>
  );
}

function SessionTransferFields({
  activeProtocol,
  draft,
  onDraftChange,
}: {
  activeProtocol: ProtocolTab;
  draft: SessionProfile;
  onDraftChange: (draft: SessionProfile) => void;
}) {
  const update = (patch: Partial<SessionProfile["transfer"]>) => onDraftChange({
    ...draft,
    transfer: { ...draft.transfer, ...patch },
  });
  const sshLike = activeProtocol === "SSH" || activeProtocol === "Tmux";

  return (
    <>
      {sshLike ? <DialogToggleField label="SFTP:" checked={draft.transfer.sftp} onChange={(sftp) => update({ sftp })} /> : null}
      {sshLike ? <DialogToggleField label="SCP:" checked={draft.transfer.scp} onChange={(scp) => update({ scp })} /> : null}
      <DialogToggleField label="XModem:" checked={draft.transfer.xmodem} onChange={(xmodem) => update({ xmodem })} />
      <DialogToggleField label="YModem:" checked={draft.transfer.ymodem} onChange={(ymodem) => update({ ymodem })} />
      <DialogToggleField label="ZModem:" checked={draft.transfer.zmodem} onChange={(zmodem) => update({ zmodem })} />
      <DialogField label="限速 B/s:">
        <input type="number" min={0} value={draft.transfer.rateLimitBytesPerSecond ?? 0} onChange={(event) => update({ rateLimitBytesPerSecond: Number(event.target.value) > 0 ? Number(event.target.value) : null })} />
      </DialogField>
      <DialogField label="默认目录:(D)">
        <input value={draft.transfer.defaultLocalDir ?? ""} onChange={(event) => update({ defaultLocalDir: event.target.value || null })} />
      </DialogField>
    </>
  );
}

function ShellProcessFields({
  draft,
  onDraftChange,
}: {
  draft: SessionProfile;
  onDraftChange: (draft: SessionProfile) => void;
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
    </>
  );
}

function TriggerFields({ draft, onDraftChange }: { draft: SessionProfile; onDraftChange: (draft: SessionProfile) => void }) {
  function setTriggers(triggers: TriggerSpec[]) {
    onDraftChange({ ...draft, triggers });
  }

  function updateTrigger(index: number, patch: Partial<TriggerSpec>) {
    setTriggers(draft.triggers.map((trigger, triggerIndex) => (
      triggerIndex === index ? { ...trigger, ...patch } : trigger
    )));
  }

  function updateAction(triggerIndex: number, actionIndex: number, action: TriggerAction) {
    const trigger = draft.triggers[triggerIndex];
    updateTrigger(triggerIndex, {
      actions: trigger.actions.map((item, index) => (index === actionIndex ? action : item)),
    });
  }

  return (
    <div className="trigger-editor">
      {draft.triggers.map((trigger, triggerIndex) => {
        const matcherValue = trigger.matcher.type === "regex" ? trigger.matcher.pattern : trigger.matcher.text;
        return (
          <section className="trigger-item" key={trigger.id}>
            <header className="trigger-item-header">
              <label className="trigger-enabled">
                <input type="checkbox" checked={trigger.enabled} onChange={(event) => updateTrigger(triggerIndex, { enabled: event.target.checked })} />
                <span>启用</span>
              </label>
              <input aria-label="触发器名称" value={trigger.label} onChange={(event) => updateTrigger(triggerIndex, { label: event.target.value })} />
              <button type="button" className="icon-button" title="删除触发器" aria-label="删除触发器" onClick={() => setTriggers(draft.triggers.filter((_, index) => index !== triggerIndex))}><Trash2 size={14} /></button>
            </header>
            <div className="trigger-matcher-row">
              <select
                aria-label="匹配类型"
                value={trigger.matcher.type}
                onChange={(event) => updateTrigger(triggerIndex, {
                  matcher: event.target.value === "regex"
                    ? { type: "regex", pattern: matcherValue }
                    : { type: "contains", text: matcherValue, case_sensitive: false },
                })}
              >
                <option value="contains">包含文本</option>
                <option value="regex">正则表达式</option>
              </select>
              <input
                aria-label="匹配内容"
                value={matcherValue}
                onChange={(event) => updateTrigger(triggerIndex, {
                  matcher: trigger.matcher.type === "regex"
                    ? { type: "regex", pattern: event.target.value }
                    : { ...trigger.matcher, text: event.target.value },
                })}
              />
              <label className="trigger-case-toggle">
                <input
                  type="checkbox"
                  checked={trigger.matcher.type === "contains" && trigger.matcher.case_sensitive}
                  disabled={trigger.matcher.type === "regex"}
                  onChange={(event) => {
                    if (trigger.matcher.type === "contains") {
                      updateTrigger(triggerIndex, { matcher: { ...trigger.matcher, case_sensitive: event.target.checked } });
                    }
                  }}
                />
                <span>区分大小写</span>
              </label>
            </div>
            <div className="trigger-action-list">
              {trigger.actions.map((action, actionIndex) => (
                <div className="trigger-action-row" key={`${trigger.id}-${actionIndex}`}>
                  <select
                    aria-label="动作类型"
                    value={action.type}
                    onChange={(event) => updateAction(triggerIndex, actionIndex, defaultTriggerAction(event.target.value as TriggerAction["type"]))}
                  >
                    <option value="timeline-mark">时间线标记</option>
                    <option value="notification">通知</option>
                    <option value="highlight">高亮</option>
                    <option value="send-text">发送文本</option>
                    <option value="local-command">本地命令</option>
                    <option value="custom-link">自定义链接</option>
                    <option value="sound">声音</option>
                  </select>
                  {action.type === "sound" ? (
                    <select aria-label="声音" value={action.name} onChange={(event) => updateAction(triggerIndex, actionIndex, { type: "sound", name: event.target.value })}>
                      <option value="bell">Bell</option>
                      <option value="chime">Chime</option>
                      <option value="alert">Alert</option>
                    </select>
                  ) : (
                    <input
                      aria-label="动作参数"
                      value={triggerActionValue(action)}
                      onChange={(event) => updateAction(triggerIndex, actionIndex, patchTriggerAction(action.type, event.target.value))}
                    />
                  )}
                  <button type="button" className="icon-button" title="删除动作" aria-label="删除动作" onClick={() => updateTrigger(triggerIndex, { actions: trigger.actions.filter((_, index) => index !== actionIndex) })}><Trash2 size={14} /></button>
                </div>
              ))}
              <button type="button" className="trigger-add-action" onClick={() => updateTrigger(triggerIndex, { actions: [...trigger.actions, defaultTriggerAction("timeline-mark")] })}><Plus size={14} />添加动作</button>
            </div>
          </section>
        );
      })}
      <button type="button" className="trigger-add" onClick={() => setTriggers([...draft.triggers, createDefaultTrigger()])}><Plus size={14} />添加触发器</button>
    </div>
  );
}

function SshAdvancedFields({
  section,
  draft,
  onDraftChange,
  proxyPasswordUpdate,
  onProxyPasswordUpdateChange,
}: {
  section: string;
  draft: SessionProfile;
  onDraftChange: (draft: SessionProfile) => void;
  proxyPasswordUpdate: ProxyPasswordUpdate;
  onProxyPasswordUpdateChange: (update: ProxyPasswordUpdate) => void;
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
    const updateSsh = (patch: Partial<typeof ssh>) => onDraftChange({
      ...draft,
      kind,
      connection: { ...ssh, ...patch, kind },
    });
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
        const response = await invokeBackend<{ secretRef: string }>("save_secret", {
          request: { secretRef: null, secret },
        });
        const patch: Partial<JumpHop> = field === "passwordSecretRef" ? { passwordSecretRef: response.secretRef } : { passphraseSecretRef: response.secretRef };
        updateJump(index, patch);
        setJumpSecretDrafts((current) => ({ ...current, [jumpSecretKey(index, field)]: "" }));
        setJumpStatus("已保存跳板凭据");
      } catch (error) {
        setJumpStatus(formatError(error));
      }
    };
    const deleteJumpSecret = (index: number, field: "passwordSecretRef" | "passphraseSecretRef") => {
      const jump = ssh.jumps[index];
      const secretRef = field === "passwordSecretRef" ? jump?.passwordSecretRef : jump?.passphraseSecretRef;
      if (!secretRef) return;
      setJumpStatus("");
      const patch: Partial<JumpHop> = field === "passwordSecretRef" ? { passwordSecretRef: null } : { passphraseSecretRef: null };
      updateJump(index, patch);
      setJumpStatus("保存 Profile 后清理未引用凭据");
    };
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
          <input type="number" min={1} max={65535} value={ssh.endpoint.port} onChange={(event) => updateSsh({ endpoint: { ...ssh.endpoint, port: Number(event.target.value) } })} />
        </DialogField>
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
        <DialogToggleField label="SSH 保活:" checked={ssh.keepaliveEnabled} onChange={(keepaliveEnabled) => updateSsh({ keepaliveEnabled })} />
        {ssh.keepaliveEnabled ? (
          <>
            <DialogField label="探测间隔(s):">
              <input
                type="number"
                min={sshConnectionBounds.keepaliveIntervalSeconds.min}
                max={sshConnectionBounds.keepaliveIntervalSeconds.max}
                value={ssh.keepaliveIntervalSeconds}
                onChange={(event) => updateSsh({ keepaliveIntervalSeconds: Number(event.target.value) })}
              />
            </DialogField>
            <DialogField label="未响应上限:">
              <input
                type="number"
                min={sshConnectionBounds.keepaliveMaxMissed.min}
                max={sshConnectionBounds.keepaliveMaxMissed.max}
                value={ssh.keepaliveMaxMissed}
                onChange={(event) => updateSsh({ keepaliveMaxMissed: Number(event.target.value) })}
              />
            </DialogField>
          </>
        ) : null}
        <DialogToggleField label="自动重连:" checked={ssh.reconnect} onChange={(reconnect) => updateSsh({ reconnect })} />
        <DialogField label="重连延迟(ms):">
          <input
            type="number"
            min={sshConnectionBounds.reconnectDelayMs.min}
            max={sshConnectionBounds.reconnectDelayMs.max}
            step={100}
            disabled={!ssh.reconnect}
            value={ssh.reconnectDelayMs}
            onChange={(event) => updateSsh({ reconnectDelayMs: Number(event.target.value) })}
          />
        </DialogField>
      </>
    );
  }

  if (section === "代理") {
    return <ProxyAdvancedFields proxy={ssh.proxy} onChange={(proxy) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, proxy } })} passwordUpdate={proxyPasswordUpdate} onPasswordUpdateChange={onProxyPasswordUpdateChange} />;
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
    const deleteSavedSecret = (field: "passwordSecretRef" | "passphraseSecretRef") => {
      const secretRef = ssh[field];
      if (!secretRef) return;
      setSecretStatus("");
      onDraftChange({ ...draft, kind, connection: { ...ssh, kind, [field]: null } });
      setSecretStatus("保存 Profile 后清理未引用凭据");
    };
    return (
      <>
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
          request: { secretRef: null, secret: vaultPrivateKey },
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
    const deleteVaultPrivateKey = () => {
      if (!firstIdentity.secretRef) return;
      setVaultBusy(true);
      setVaultStatus("");
      updateIdentity({ secretRef: null });
      setVaultStatus("保存 Profile 后清理未引用私钥");
      setVaultBusy(false);
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

  return null;
}

function ProxyAdvancedFields({
  proxy,
  onChange,
  passwordUpdate,
  onPasswordUpdateChange,
}: {
  proxy: ProxyConfig;
  onChange: (proxy: ProxyConfig) => void;
  passwordUpdate: ProxyPasswordUpdate;
  onPasswordUpdateChange: (update: ProxyPasswordUpdate) => void;
}) {
  const update = (patch: Partial<ProxyConfig>) => onChange({ ...proxy, ...patch });
  const password = passwordUpdate?.action === "set" ? passwordUpdate.password : "";
  const passwordPendingClear = passwordUpdate?.action === "clear";
  return (
    <>
      <DialogToggleField label="启用代理:" checked={proxy.enabled} onChange={(enabled) => update({ enabled })} />
      {proxy.enabled ? (
        <>
          <DialogField label="协议:">
            <select value={proxy.kind} onChange={(event) => update({ kind: event.target.value as ProxyConfig["kind"] })}>
              <option value="socks5">SOCKS5</option>
              <option value="http-connect">HTTP CONNECT</option>
            </select>
          </DialogField>
          <DialogField label="代理主机:">
            <input value={proxy.host} onChange={(event) => update({ host: event.target.value })} />
          </DialogField>
          <DialogField label="代理端口:">
            <input type="number" min={1} max={65535} value={proxy.port} onChange={(event) => update({ port: Number(event.target.value) })} />
          </DialogField>
          <DialogField label="代理用户:">
            <input value={proxy.username} autoComplete="username" onChange={(event) => update({ username: event.target.value })} />
          </DialogField>
          <DialogField label="代理密码:">
            <form className="proxy-password-control" onSubmit={(event) => event.preventDefault()}>
              <input type="text" name="username" autoComplete="username" value={proxy.username} readOnly hidden aria-hidden="true" tabIndex={-1} />
              <input
                type="password"
                name="password"
                autoComplete="new-password"
                value={password}
                placeholder={passwordPendingClear ? "保存后移除" : proxy.passwordSecretRef ? "已安全保存" : "未保存"}
                onChange={(event) => onPasswordUpdateChange(event.target.value ? { action: "set", password: event.target.value } : null)}
              />
              <button
                type="button"
                className="icon-button"
                title="移除已保存的代理密码"
                aria-label="移除已保存的代理密码"
                disabled={passwordPendingClear || (!proxy.passwordSecretRef && passwordUpdate?.action !== "set")}
                onClick={() => onPasswordUpdateChange({ action: "clear" })}
              >
                <X size={14} />
              </button>
            </form>
          </DialogField>
        </>
      ) : null}
    </>
  );
}

function TcpLikeAdvancedFields({
  protocol,
  section,
  draft,
  onDraftChange,
  proxyPasswordUpdate,
  onProxyPasswordUpdateChange,
}: {
  protocol: "Telnet" | "Tcp";
  section: string;
  draft: SessionProfile;
  onDraftChange: (draft: SessionProfile) => void;
  proxyPasswordUpdate: ProxyPasswordUpdate;
  onProxyPasswordUpdateChange: (update: ProxyPasswordUpdate) => void;
}) {
  const kind = protocol === "Telnet" ? "telnet" : "tcp";
  const tcp = draft.connection.kind === kind ? draft.connection : createTcpConnection(kind);

  if (section === "连接") {
    const updateTcp = (patch: Partial<typeof tcp>) => onDraftChange({
      ...draft,
      kind,
      connection: { ...tcp, ...patch, kind },
    });
    return (
      <>
        <DialogField label="主机:(H)">
          <input value={tcp.host} onChange={(event) => updateTcp({ host: event.target.value })} />
        </DialogField>
        <DialogField label="端口:(P)">
          <input type="number" min={1} max={65535} value={tcp.port} onChange={(event) => updateTcp({ port: Number(event.target.value) })} />
        </DialogField>
        <DialogToggleField label="自动重连:" checked={tcp.reconnect} onChange={(reconnect) => updateTcp({ reconnect })} />
        <DialogField label="重连延迟(ms):">
          <input
            type="number"
            min={tcpConnectionBounds.reconnectDelayMs.min}
            max={tcpConnectionBounds.reconnectDelayMs.max}
            step={100}
            disabled={!tcp.reconnect}
            value={tcp.reconnectDelayMs}
            onChange={(event) => updateTcp({ reconnectDelayMs: Number(event.target.value) })}
          />
        </DialogField>
        <DialogToggleField label="TCP KeepAlive:" checked={tcp.keepaliveEnabled} onChange={(keepaliveEnabled) => updateTcp({ keepaliveEnabled })} />
        {tcp.keepaliveEnabled ? (
          <>
            <DialogField label="空闲时间(s):">
              <input
                type="number"
                min={tcpConnectionBounds.keepaliveIdleSeconds.min}
                max={tcpConnectionBounds.keepaliveIdleSeconds.max}
                value={tcp.keepaliveIdleSeconds}
                onChange={(event) => updateTcp({ keepaliveIdleSeconds: Number(event.target.value) })}
              />
            </DialogField>
            <DialogField label="探测间隔(s):">
              <input
                type="number"
                min={tcpConnectionBounds.keepaliveIntervalSeconds.min}
                max={tcpConnectionBounds.keepaliveIntervalSeconds.max}
                value={tcp.keepaliveIntervalSeconds}
                onChange={(event) => updateTcp({ keepaliveIntervalSeconds: Number(event.target.value) })}
              />
            </DialogField>
            <DialogField label="失败次数:">
              <input
                type="number"
                min={tcpConnectionBounds.keepaliveRetries.min}
                max={tcpConnectionBounds.keepaliveRetries.max}
                value={tcp.keepaliveRetries}
                onChange={(event) => updateTcp({ keepaliveRetries: Number(event.target.value) })}
              />
            </DialogField>
          </>
        ) : null}
        {protocol === "Telnet" ? (
          <>
            <DialogToggleField label="BINARY:" checked={tcp.telnetBinary} onChange={(telnetBinary) => updateTcp({ telnetBinary })} />
            <DialogToggleField label="NAWS:" checked={tcp.telnetNaws} onChange={(telnetNaws) => updateTcp({ telnetNaws })} />
          </>
        ) : null}
      </>
    );
  }

  return <ProxyAdvancedFields proxy={tcp.proxy} onChange={(proxy) => onDraftChange({ ...draft, kind, connection: { ...tcp, kind, proxy } })} passwordUpdate={proxyPasswordUpdate} onPasswordUpdateChange={onProxyPasswordUpdateChange} />;
}

function SerialAdvancedFields({
  draft,
  serialPorts,
  onDraftChange,
}: {
  draft: SessionProfile;
  serialPorts: string[];
  onDraftChange: (draft: SessionProfile) => void;
}) {
  const serial = draft.connection.kind === "serial" ? draft.connection : createSerialConnection();
  const update = (patch: Partial<ReturnType<typeof createSerialConnection>>) => onDraftChange({ ...draft, kind: "serial", connection: { ...serial, ...patch } });

  return (
    <>
      <DialogField label="串口:(S)">
        <select value={serial.port} onChange={(event) => update({ port: event.target.value })}>
          {serialPortOptions(serial.port, serialPorts).map((option) => (
            <option key={option || "blank"} value={option}>
              {option || "选择串口"}
            </option>
          ))}
        </select>
      </DialogField>
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
      <DialogToggleField label="自动重连:" checked={serial.reconnect} onChange={(reconnect) => update({ reconnect })} />
      <DialogField label="重连延迟(ms):">
        <input
          type="number"
          min={serialConnectionBounds.reconnectDelayMs.min}
          max={serialConnectionBounds.reconnectDelayMs.max}
          step={100}
          disabled={!serial.reconnect}
          value={serial.reconnectDelayMs}
          onChange={(event) => update({ reconnectDelayMs: Number(event.target.value) })}
        />
      </DialogField>
      <DialogToggleField label="接收空闲超时:" checked={serial.receiveIdleTimeoutEnabled} onChange={(receiveIdleTimeoutEnabled) => update({ receiveIdleTimeoutEnabled })} />
      {serial.receiveIdleTimeoutEnabled ? (
        <DialogField label="空闲上限(s):">
          <input
            type="number"
            min={serialConnectionBounds.receiveIdleTimeoutSeconds.min}
            max={serialConnectionBounds.receiveIdleTimeoutSeconds.max}
            value={serial.receiveIdleTimeoutSeconds}
            onChange={(event) => update({ receiveIdleTimeoutSeconds: Number(event.target.value) })}
          />
        </DialogField>
      ) : null}
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

function SettingInput({ label, value, type = "text", placeholder, min, max, step, onChange }: { label: string; value: string | number; type?: string; placeholder?: string; min?: number; max?: number; step?: number; onChange: (value: string) => void }) {
  return (
    <label className="setting-row">
      <span>{label}</span>
      <input type={type} value={value} placeholder={placeholder} min={min} max={max} step={step} onChange={(event) => onChange(event.target.value)} />
    </label>
  );
}

type SettingSelectOption = string | { value: string; label: string };

function SettingSelect({ label, value, options, disabled = false, onChange }: { label: string; value: string; options: readonly SettingSelectOption[]; disabled?: boolean; onChange: (value: string) => void }) {
  return (
    <label className="setting-row">
      <span>{label}</span>
      <select aria-label={label} value={value} disabled={disabled} onChange={(event) => onChange(event.target.value)}>
        {options.map((option) => {
          const optionValue = typeof option === "string" ? option : option.value;
          const optionLabel = typeof option === "string" ? option || "未指定" : option.label;
          return <option key={optionValue || "blank"} value={optionValue}>
            {optionLabel}
          </option>
        })}
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
    startupMode: "last",
    startupSessions: ["", "", "", ""],
    lockOnIdle: false,
    lockScreenTimeoutMinutes: 30,
    requireMasterPassword: false,
    completionEnabled: true,
    oneKeyCompletionEnabled: true,
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
      retentionDays: 0,
    },
    triggers: [],
    transfer: { sftp: true, scp: true, xmodem: true, ymodem: true, zmodem: true, rateLimitBytesPerSecond: null, defaultLocalDir: null },
  };
}

function prepareSessionProfile(profile: SessionProfile): SessionProfile {
  const id = profile.id && profile.id !== "draft" ? profile.id : createSessionId();
  const name = profile.name.trim() || defaultSessionName(profile);
  const metadata = normalizeSessionProfileMetadata({ ...profile, name }, name);
  const connection = normalizeConnectionConfig(profile.connection, id);
  return {
    ...profile,
    id,
    name: metadata.name,
    kind: connection.kind,
    connection,
    group: metadata.group,
    tags: metadata.tags,
    terminal: {
      ...normalizeTerminalProfileSettings(profile.terminal),
      theme: normalizeTerminalTheme(profile.terminal.theme),
    },
    logging: {
      ...profile.logging,
      pathTemplate: profile.logging.pathTemplate.trim() || "{profile}/{date}/{session}.jsonl",
      retentionDays: Math.min(3650, Math.max(0, Math.trunc(profile.logging.retentionDays ?? 0))),
    },
  };
}

function normalizeConnectionConfig(connection: ConnectionConfig, profileId: string): ConnectionConfig {
  if (connection.kind === "tcp" || connection.kind === "telnet") {
    return normalizeTcpConnectionSettings({
      ...connection,
      host: connection.host.trim(),
      port: Number.isFinite(connection.port) ? Math.min(65535, Math.max(0, Math.trunc(connection.port))) : 0,
      proxy: normalizeProxyConfig(connection.proxy),
    });
  }
  if (connection.kind === "serial") {
    return normalizeSerialConnectionSettings({
      ...connection,
      port: connection.port.trim(),
    });
  }
  if (connection.kind !== "ssh" && connection.kind !== "tmux") {
    return connection;
  }
  const normalized = normalizeSshConnectionSettings({
    ...connection,
    proxy: normalizeProxyConfig(connection.proxy),
  });
  const alias = normalized.hostKeyPolicy.alias?.trim();
  return {
    ...normalized,
    jumps: normalized.jumps
      .map((jump) => ({
        host: jump.host.trim(),
        port: Number.isFinite(jump.port) && jump.port > 0 ? Math.min(65535, Math.trunc(jump.port)) : 22,
        username: jump.username.trim() || normalized.username.trim(),
        passwordSecretRef: jump.passwordSecretRef?.trim() || null,
        passphraseSecretRef: jump.passphraseSecretRef?.trim() || null,
        identityRef: jump.identityRef?.trim() || null,
        hostKeyPolicy: normalizeOptionalHostKeyPolicy(jump.hostKeyPolicy),
      }))
      .filter((jump) => jump.host),
    hostKeyPolicy: {
      ...normalized.hostKeyPolicy,
      alias: alias || profileId,
    },
    trustedHostKeys: normalized.trustedHostKeys.filter((key) => key.scope !== "profile" || !key.profileId || key.profileId === profileId),
    identityPolicy: {
      ...normalized.identityPolicy,
      authOrder: normalized.identityPolicy.authOrder.map(normalizeAuthMethod).filter((method, index, methods) => methods.indexOf(method) === index),
      lastSuccessful: normalized.identityPolicy.lastSuccessful ? normalizeAuthMethod(normalized.identityPolicy.lastSuccessful) : null,
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

async function playTriggerSound(name: string) {
  const AudioContextClass = window.AudioContext;
  if (!AudioContextClass) return;
  const context = new AudioContextClass();
  const oscillator = context.createOscillator();
  const gain = context.createGain();
  const frequency = name === "alert" ? 880 : name === "chime" ? 660 : 440;
  oscillator.frequency.value = frequency;
  oscillator.type = name === "alert" ? "square" : "sine";
  gain.gain.setValueAtTime(0.0001, context.currentTime);
  gain.gain.exponentialRampToValueAtTime(0.12, context.currentTime + 0.015);
  gain.gain.exponentialRampToValueAtTime(0.0001, context.currentTime + 0.22);
  oscillator.connect(gain);
  gain.connect(context.destination);
  oscillator.start();
  oscillator.stop(context.currentTime + 0.24);
  oscillator.addEventListener("ended", () => void context.close(), { once: true });
  await context.resume();
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
      request: { secretRef: null, secret: credentials.password },
    });
    connection = { ...connection, passwordSecretRef: response.secretRef };
  }
  if (credentials.savePassphrase && credentials.passphrase) {
    const response = await invokeBackend<{ secretRef: string }>("save_secret", {
      request: { secretRef: null, secret: credentials.passphrase },
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

async function openDetachedPaneWindow(request: DetachedPaneRequest, sessionName: string): Promise<void> {
  const path = buildDetachedPanePath(request);
  if (!isBackendAvailable()) {
    const popup = window.open(path, request.windowId, "popup,width=960,height=680,resizable=yes");
    if (!popup) throw new Error("浏览器阻止了独立窗口，请允许 PortMate 打开弹出窗口。");
    return;
  }
  await new Promise<void>((resolve, reject) => {
    let settled = false;
    const finish = (error?: Error) => {
      if (settled) return;
      settled = true;
      window.clearTimeout(timeout);
      if (error) reject(error);
      else resolve();
    };
    const timeout = window.setTimeout(() => finish(new Error("创建独立窗口超时")), 8_000);
    const child = new WebviewWindow(request.windowId, {
      url: path,
      title: `${sessionName} - PortMate`,
      center: true,
      width: 960,
      height: 680,
      minWidth: 640,
      minHeight: 400,
      preventOverflow: true,
    });
    void child.once("tauri://created", () => finish());
    void child.once<unknown>("tauri://error", (event) => finish(new Error(formatError(event.payload))));
  });
}

function loadWorkspaceSnapshot(): WorkspaceSnapshot {
  const stored = loadLocalValue<unknown>(WORKSPACE_STORAGE_KEY, null);
  if (stored) return sanitizeWorkspaceSnapshot(stored);
  return sanitizeWorkspaceSnapshot({
    version: 1,
    layout: loadLocalValue<unknown>("portmate.workspaceLayout", "single"),
    paneIds: loadLocalValue<unknown>("portmate.paneIds", []),
    activeId: loadLocalValue<unknown>("portmate.activeId", ""),
    tabColors: loadLocalValue<unknown>("portmate.tabColors", {}),
  });
}

function normalizeTerminalPrefs(value: unknown): TerminalPrefs {
  const defaults = createTerminalPrefs();
  const source = value && typeof value === "object" && !Array.isArray(value)
    ? value as Partial<TerminalPrefs>
    : {};
  const historyLimit = normalizeCommandHistoryInteger(source.historyLimit, 1, MAX_COMMAND_HISTORY_LIMIT, MAX_COMMAND_HISTORY_LIMIT);
  const historyRetentionDays = normalizeCommandHistoryInteger(source.historyRetentionDays, 0, MAX_COMMAND_HISTORY_RETENTION_DAYS, 30);
  return {
    startupMode: source.startupMode === "none" || source.startupMode === "specific" || source.startupMode === "last"
      ? source.startupMode
      : defaults.startupMode,
    startupSessions: normalizeTerminalStartupSessionIds(source.startupSessions),
    lockOnIdle: booleanPreference(source.lockOnIdle, defaults.lockOnIdle),
    lockScreenTimeoutMinutes: normalizeScreenLockTimeoutMinutes(source.lockScreenTimeoutMinutes),
    requireMasterPassword: booleanPreference(source.requireMasterPassword, defaults.requireMasterPassword),
    completionEnabled: booleanPreference(source.completionEnabled, defaults.completionEnabled),
    oneKeyCompletionEnabled: booleanPreference(source.oneKeyCompletionEnabled, defaults.oneKeyCompletionEnabled),
    completionCommandNames: booleanPreference(source.completionCommandNames, defaults.completionCommandNames),
    completionCommandOptions: booleanPreference(source.completionCommandOptions, defaults.completionCommandOptions),
    completionCommandArgs: booleanPreference(source.completionCommandArgs, defaults.completionCommandArgs),
    completionHistory: booleanPreference(source.completionHistory, defaults.completionHistory),
    completionQuickCommands: booleanPreference(source.completionQuickCommands, defaults.completionQuickCommands),
    completionTriggerChars: stringPreference(source.completionTriggerChars, ["1 字符", "2 字符", "3 字符"], defaults.completionTriggerChars),
    completionListHeight: stringPreference(source.completionListHeight, ["5 行", "7 行", "10 行"], defaults.completionListHeight),
    completionPreviewMode: stringPreference(source.completionPreviewMode, ["无处", "输入框", "列表顶部"], defaults.completionPreviewMode),
    historyEnabled: booleanPreference(source.historyEnabled, defaults.historyEnabled),
    historyRetentionDays: String(historyRetentionDays),
    historyLimit: String(historyLimit),
    mouseReporting: booleanPreference(source.mouseReporting, defaults.mouseReporting),
    mouseCopyOnSelect: booleanPreference(source.mouseCopyOnSelect, defaults.mouseCopyOnSelect),
  };
}

function booleanPreference(value: unknown, fallback: boolean): boolean {
  return typeof value === "boolean" ? value : fallback;
}

function stringPreference(value: unknown, allowed: readonly string[], fallback: string): string {
  return typeof value === "string" && allowed.includes(value) ? value : fallback;
}

function normalizeCommandHistoryInteger(
  value: unknown,
  minimum: number,
  maximum: number,
  fallback: number,
): number {
  const parsed = typeof value === "number" ? value : typeof value === "string" ? Number(value) : Number.NaN;
  if (!Number.isFinite(parsed)) return fallback;
  return Math.min(maximum, Math.max(minimum, Math.trunc(parsed)));
}

function loadTerminalPrefs(): TerminalPrefs {
  return normalizeTerminalPrefs(loadLocalValue<unknown>("portmate.terminalPrefs", null));
}

function loadInitialScreenLockState(lockOnStartup = false): ScreenLockState {
  try {
    const raw = window.localStorage.getItem(SCREEN_LOCK_STORAGE_KEY);
    const decoded = decodeStoredScreenLockMarker(raw);
    if (!decoded) {
      return lockOnStartup ? createStartupScreenLockState() : null;
    }
    return {
      reason: "restored",
      lockedAt: decoded.marker.lockedAt,
      mode: "preparing",
      restoreVaultLocked: loadScreenLockVaultRestoreState(),
      repairMarker: decoded.recovered,
      message: "",
    };
  } catch {
    return lockOnStartup ? createStartupScreenLockState() : null;
  }
}

function createStartupScreenLockState(): NonNullable<ScreenLockState> {
  return {
    reason: "startup",
    lockedAt: Date.now(),
    mode: "preparing",
    restoreVaultLocked: null,
    repairMarker: false,
    message: "",
  };
}

const SCREEN_LOCK_VAULT_RESTORE_STORAGE_KEY = "portmate.screenLock.vaultRestore.v1";

function loadScreenLockVaultRestoreState(): boolean | null {
  try {
    const value = window.sessionStorage.getItem(SCREEN_LOCK_VAULT_RESTORE_STORAGE_KEY);
    if (value === "locked") return true;
    if (value === "unlocked") return false;
  } catch {
    // The current lock can still use its in-memory restore state.
  }
  return null;
}

function saveScreenLockVaultRestoreState(restoreLocked: boolean) {
  try {
    window.sessionStorage.setItem(SCREEN_LOCK_VAULT_RESTORE_STORAGE_KEY, restoreLocked ? "locked" : "unlocked");
  } catch {
    // Reload restoration is unavailable, but the current lock remains protected.
  }
}

function clearScreenLockVaultRestoreState() {
  try {
    window.sessionStorage.removeItem(SCREEN_LOCK_VAULT_RESTORE_STORAGE_KEY);
  } catch {
    // A stale session-only hint cannot unlock the workspace or the vault.
  }
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
    ...serialConnectionDefaults,
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
    proxy: { ...proxyDefaults },
    ...tcpConnectionDefaults,
  };
}

function createSshConnection(): Extract<ConnectionConfig, { kind: "ssh" | "tmux" }> {
  return {
    kind: "ssh",
    endpoint: { host: "", port: 22 },
    username: "",
    reconnect: true,
    ...sshConnectionDefaults,
    proxy: { ...proxyDefaults },
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

function formatSysmonUptime(seconds: number) {
  const wholeSeconds = Math.max(0, Math.trunc(seconds));
  const days = Math.floor(wholeSeconds / 86_400);
  const hours = Math.floor((wholeSeconds % 86_400) / 3_600);
  const minutes = Math.floor((wholeSeconds % 3_600) / 60);
  if (days) return `${days}d ${hours}h`;
  if (hours) return `${hours}h ${minutes}m`;
  if (minutes) return `${minutes}m ${wholeSeconds % 60}s`;
  return `${wholeSeconds}s`;
}

function sysmonPercentLevel(percent: number) {
  if (percent >= 80) return "critical";
  if (percent >= 60) return "warning";
  return "normal";
}

function formatSysmonTrendValue(snapshot: SysmonSnapshot, mode: SysmonTrendMode, series: 0 | 1) {
  const value = sysmonTrendValue(snapshot, mode, series);
  return mode === "usage" ? `${value.toFixed(1)}%` : formatSysmonTrendRate(value);
}

function formatSysmonTrendAxis(value: number, mode: SysmonTrendMode) {
  if (mode === "usage") return `${Math.round(value)}%`;
  if (value >= 1024) return `${(value / 1024).toFixed(value >= 10_240 ? 0 : 1)}M`;
  return `${Math.round(value)}K`;
}

function formatSysmonTrendRate(kibibytesPerSecond: number) {
  if (kibibytesPerSecond >= 1024) {
    return `${(kibibytesPerSecond / 1024).toFixed(1)} MiB/s`;
  }
  return `${kibibytesPerSecond.toFixed(1)} KiB/s`;
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
