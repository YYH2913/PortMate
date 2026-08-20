import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties, DragEvent as ReactDragEvent, FormEvent, MouseEvent as ReactMouseEvent, PointerEvent as ReactPointerEvent, SetStateAction } from "react";
import { emitTo, listen } from "@tauri-apps/api/event";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import {
  Activity,
  AlertCircle,
  ArrowRightLeft,
  Check,
  Clock3,
  Download,
  Files,
  Folder,
  Lock,
  LoaderCircle,
  Maximize2,
  Minimize2,
  PanelBottom,
  PanelLeft,
  PanelRight,
  Pencil,
  Play,
  Plus,
  RefreshCw,
  Search,
  SendHorizontal,
  Settings,
  SlidersHorizontal,
  Square,
  SquareTerminal,
  Trash2,
  Unlock,
  X,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { callBackend, emptyAudit, emptyGrants, emptyHostKeys, emptyLogs, emptySessions, emptyTransfers, invokeBackend, isBackendAvailable } from "./api";
import { waitForChildWindowReady } from "./child-window-launch";
import type { CommandHistoryEntry } from "./command-history-state";
import { mergeTransfers } from "./transfer-state";
import { addDismissedTransferId } from "./transfer-visibility";
import { hostKeyProfileSnapshotMatches } from "./host-key-profile-state";
import { KeyedRequestGate } from "./keyed-request-gate";
import { MCP_APPROVAL_EVENT, mergeMcpApprovals } from "./mcp-approval-state";
import { menuGroups, menuItemDisabled } from "./menu-capabilities";
import type { MenuCapabilityContext, MenuItem } from "./menu-capabilities";
import {
  hasActiveInteractionLayer,
  INTERACTION_LAYER_DISMISS_EVENT,
} from "./modal-interaction-boundary";
import { buildDetachedPanePath, DETACHED_PANE_EVENT, DETACHED_PANE_RESULT_EVENT, DETACHED_PANE_RESULT_MESSAGE_TYPE, normalizeDetachedPaneCommand, normalizeDetachedPaneMessage, SESSION_PROFILE_DELETED_EVENT, SESSION_PROFILE_UPDATED_EVENT } from "./detached-pane-state";
import type { DetachedPaneCommand, DetachedPaneRequest, DetachedPaneResult } from "./detached-pane-state";
import { detachedPaneWindowGeometryKey, placeAndTrackChildWindow } from "./window-geometry";
import { buildWorkspaceWindowPath } from "./workspace-window-route";
import { formatBytes, formatDuration, formatEventClock } from "./display-formatters";
import { normalizeProxyConfig } from "./proxy-settings";
import type { ProxyPasswordUpdate } from "./proxy-settings";
import { normalizeQuickCommandLibrary, QUICK_BAR_VISIBLE_STORAGE_KEY, QUICK_COMMAND_STORAGE_KEY, quickCommandDispatch } from "./quick-command-state";
import type { QuickCommand } from "./quick-command-state";
import { normalizeSerialConnectionSettings } from "./serial-connection-settings";
import type { SerialAnalyzerRequest } from "./serial-analyzer-route";
import type { SearchDialogState } from "./SearchDialog";
import { normalizeSessionProfileMetadata } from "./session-settings-state";
import { applyPuttyImportTerminal, createOpenSshImportConnection, createPuttyImportConnection, createSerialConnection, createShellImportConnection, formatSshTarget } from "./session-profile-helpers";
import type { OpenSshImportCandidate } from "./openssh-config-import";
import type { PuttySessionImportCandidate } from "./putty-session-import";
import type { ShellSessionImportCandidate } from "./shell-session-import";
import { sessionConnectionAction, sessionRuntimeHealthDescription, transitionSessionRuntimeStatus } from "./session-runtime-state";
import { filterSerialCaptureFrames, mergeSerialCaptureSnapshot, serialCaptureAscii, serialCaptureHex } from "./serial-capture-state";
import type { SerialCaptureDirectionFilter } from "./serial-capture-state";
import { createScreenLockMarker, decodeStoredScreenLockMarker, isScreenLockShortcut, MAX_SCREEN_LOCK_TIMEOUT_MINUTES, MIN_SCREEN_LOCK_TIMEOUT_MINUTES, normalizeScreenLockTimeoutMinutes, SCREEN_LOCK_STORAGE_KEY, shouldAutoLockScreen } from "./screen-lock-state";
import type { ScreenLockReason } from "./screen-lock-state";
import { normalizeSshConnectionSettings } from "./ssh-connection-settings";
import { defaultSyncInputSettings, normalizeSyncInputSettings, resolveSyncInputTargets, SyncInputDispatcher } from "./sync-input-state";
import type { SyncInputOrigin, SyncInputSettings } from "./sync-input-state";
import { requestTerminalFreeInput } from "./terminal-free-input";
import { requestTerminalTextExport } from "./terminal-export-event";
import type { TerminalTextExportSource } from "./terminal-export-event";
import {
  chooseTerminalTextExportPath,
  normalizeTerminalExportDirectory,
  terminalTextExportFileName,
} from "./terminal-export-path";
import type { TerminalBufferAction } from "./terminal-buffer-event";
import type { TerminalSelectionAction } from "./terminal-selection-event";
import { DEFAULT_TERMINAL_FONT_FAMILY, normalizeTerminalProfileSettings, normalizeTerminalStartupSessionIds } from "./terminal-settings-state";
import { requestTerminalGotoLine } from "./terminal-goto-line-event";
import { listenTerminalByteEvents } from "./terminal-byte-events";
import { terminalKeyModeLabel, toggleTerminalInsertNormalMode } from "./terminal-key-mode";
import type { TerminalKeyMode } from "./terminal-key-mode";
import { requestTerminalSearch } from "./terminal-search";
import { normalizeTerminalTheme } from "./terminal-theme";
import { formatTcpConnectionTarget, normalizeTcpConnectionSettings } from "./tcp-connection-settings";
import { defaultWorkspaceKeymap, LEGACY_WORKSPACE_KEYMAP_STORAGE_KEY, normalizeWorkspaceKeymap, resolveWorkspaceHotkeySequence, WORKSPACE_KEY_CHORD_TIMEOUT_MS, WORKSPACE_KEYMAP_STORAGE_KEY } from "./workspace-hotkeys";
import type { WorkspaceKeymap } from "./workspace-hotkeys";
import type { WorkspaceViewContextAction } from "./WorkspaceViewContextMenu";
import { activateWorkspaceDockPanel, activeWorkspaceDockPanel, clampWorkspaceDockSize, isWorkspaceFocusModeShortcut, LEGACY_WORKSPACE_PANEL_STORAGE_KEY, moveWorkspacePanelToDock, normalizeWorkspaceDockLayout, normalizeWorkspaceDockSizes, normalizeWorkspacePanelVisibility, resolveWorkspacePanelVisibility, setWorkspaceDockSize, setWorkspacePanelVisibility, visibleWorkspaceDockPanels, workspaceDockEffectiveSize, workspaceDockIds, workspaceDockPanelIds, workspaceDockSizeLimits, WORKSPACE_PANEL_STORAGE_KEY } from "./workspace-panel-state";
import type { WorkspaceDockId, WorkspaceDockLayout, WorkspaceDockPanelId, WorkspaceDockSizes, WorkspacePanelId } from "./workspace-panel-state";
import { workspaceSplitDirectionForVisualOrientation, workspaceViewContextCapabilities } from "./workspace-view-context-state";
import { commitWorkspaceViewDetach, commitWorkspaceViewReattach } from "./workspace-detach-state";
import { activateWorkspacePaneSession, activateWorkspacePaneView, addWorkspacePaneSession, canSplitWorkspacePane, createWorkspaceNodeId, createWorkspacePane, duplicateWorkspacePaneView, emptyWorkspaceSnapshot, findWorkspacePane, findWorkspacePaneBySession, findWorkspacePaneInDirection, insertWorkspacePaneView, MAX_WORKSPACE_DEPTH, MAX_WORKSPACE_GROUP_TABS, MAX_WORKSPACE_PANES, MAX_WORKSPACE_SPLIT_RATIO, mergeWorkspacePaneGroups, MIN_WORKSPACE_SPLIT_RATIO, moveWorkspacePaneView, moveWorkspacePaneViewToNewGroup, reconcileWorkspaceSnapshot, removeWorkspacePane, removeWorkspacePaneView, renameWorkspacePaneView, replaceWorkspacePaneSession, resetWorkspaceTerminalKeyModes, resolveStartupSessionIds, sanitizeWorkspaceSnapshot, setWorkspacePaneViewColor, setWorkspacePaneViewKeyMode, splitWorkspacePane, splitWorkspacePaneViewToGroup, swapWorkspacePanes, updateWorkspaceSplitRatio, workspacePaneActiveView, workspacePaneLeaves, workspacePaneViewAtOffset } from "./workspace-state";
import type { StartupMode, WorkspaceNode, WorkspacePaneDirection, WorkspacePaneNode, WorkspaceSnapshot, WorkspaceSplitDirection, WorkspaceSplitNode, WorkspaceSplitPlacement, WorkspaceView } from "./workspace-state";
import type { AuditRecord, CommandHistorySnapshot, ConnectionConfig, DeleteSessionProfileResponse, ExportSerialCaptureResult, ExportTerminalTextResult, HostKeyObservation, HostKeyPolicy, HostKeyScanResult, HostKeyStore, McpApprovalRequest, McpGrant, OneKeySummary, SerialCaptureFrame, SerialCaptureSnapshot, SessionEvent, SessionProfile, SessionStatus, SessionSummary, SysmonSnapshot, TransferTask, TriggerEffect, TrustedHostKey } from "./types";
import { sshOneKeysForSession } from "./one-key-login-state";
import type { ConnectionCredentials, CredentialPromptState } from "./CredentialDialog";
import { stageConnectionCredentials } from "./session-credential-state";
import type { OneKeyPromptField } from "./one-key-completion-state";
import { deleteSessionProfileFromClientState } from "./session-profile-delete-state";
import type { SessionContextAction, TerminalContextAction } from "./ContextMenus";

const LazyTerminalCanvas = lazy(() => import("./TerminalCanvas"));
const LazyQuickCommandDialog = lazy(() => import("./QuickCommandDialog"));
const LazyCustomScriptDialog = lazy(() => import("./CustomScriptDialog"));
const LazyOneKeyDialog = lazy(() => import("./OneKeyDialog"));
const LazySearchDialog = lazy(() => import("./SearchDialog"));
const LazyTmuxDialog = lazy(() => import("./TmuxDialog"));
const LazyMcpDialog = lazy(() => import("./McpDialog"));
const LazyMcpApprovalDialog = lazy(() => import("./McpApprovalDialog"));
const LazyCredentialDialog = lazy(() => import("./CredentialDialog"));
const LazyNoticeDialog = lazy(() => import("./NoticeDialog"));
const LazySessionContextMenu = lazy(() => import("./ContextMenus").then(({ SessionContextMenu }) => ({ default: SessionContextMenu })));
const LazyTerminalContextMenu = lazy(() => import("./ContextMenus").then(({ TerminalContextMenu }) => ({ default: TerminalContextMenu })));
const LazySessionExplorerPanel = lazy(() => import("./WorkspaceUtilityPanels").then(({ SessionExplorerPanel }) => ({ default: SessionExplorerPanel })));
const LazyCommandHistoryList = lazy(() => import("./WorkspaceUtilityPanels").then(({ CommandHistoryList }) => ({ default: CommandHistoryList })));
const LazyWorkspaceViewContextMenu = lazy(() => import("./WorkspaceViewContextMenu"));
const LazyWorkspaceViewRenameDialog = lazy(() => import("./WorkspaceViewRenameDialog"));
const LazyTransferDialog = lazy(() => import("./TransferDialog"));
const LazyTunnelDialog = lazy(() => import("./TunnelDialog"));
const LazySysmonDialog = lazy(() => import("./SysmonDialog"));
const LazySysmonSidebar = lazy(() => import("./SysmonSidebar"));
const LazyLogManagerDialog = lazy(() => import("./LogManagerDialog"));
const LazyKeyManagerDialog = lazy(() => import("./KeyManagerDialog"));
const LazyTerminalSettingsDialog = lazy(() => import("./TerminalSettingsDialog"));
const LazySessionSettingsDialog = lazy(() => import("./SessionSettingsDialog"));
const LazySessionImportDialog = lazy(() => import("./SessionImportDialog"));
const LazyFileManagerPanel = lazy(() => import("./FileManagerPanel"));

const WORKSPACE_STORAGE_KEY = "portmate.workspace.v1";
const WORKSPACE_WINDOW_WIDTH = 1280;
const WORKSPACE_WINDOW_HEIGHT = 820;
const WORKSPACE_WINDOW_MIN_WIDTH = 1100;
const WORKSPACE_WINDOW_MIN_HEIGHT = 720;
const MAX_CLOSED_WORKSPACE_VIEWS = 32;
const COMMAND_HISTORY_STORAGE_KEY = "portmate.commandHistory";
const MAX_COMMAND_HISTORY_LIMIT = 10_000;
const MAX_COMMAND_HISTORY_RETENTION_DAYS = 3_650;
const MAX_RESOLVED_MCP_APPROVAL_IDS = 256;
const COMMAND_HISTORY_UPDATED_EVENT = "portmate-command-history-updated";
type StartupHydrationDomain = "transfers" | "audit" | "grants" | "host-keys" | "one-keys" | "serial-ports";
const workspaceUtilityIcons = { Folder, Search, X };
const workspaceDockPanelMeta: Record<WorkspaceDockPanelId, { label: string; icon: LucideIcon }> = {
  explorer: { label: "资源管理器", icon: Folder },
  fileManager: { label: "文件管理器", icon: Files },
  history: { label: "历史命令", icon: Clock3 },
  sysmon: { label: "Sysmon", icon: Activity },
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
  "Sysmon 侧栏": "sysmon",
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
  "Insert 模式": "remote",
  本地模式: "local",
  本地编辑: "normal",
  "Normal 模式": "command",
};

type SettingsDialog = "terminal" | "session" | null;
type SessionSettingsMode = "create" | "edit";
type UtilityDialog = "transfer" | "tunnel" | "tmux" | "sysmon" | "search" | "logs" | "keys" | "mcp" | "one-keys" | "quick-commands" | "custom-scripts" | "session-import" | null;
type ConnectionInteraction = "interactive" | "silent";
type TerminalPrefs = ReturnType<typeof createTerminalPrefs>;
type NoticeState = { title: string; message: string; link?: string } | null;
type WorkspaceGroupMoveRequest = { paneId: string; mode: "view" | "group" } | null;
type WorkspaceViewRenameRequest = { paneId: string; viewId: string; value: string; sessionName: string } | null;
type WorkspaceViewContextMenuState = { x: number; y: number; paneId: string; viewId: string } | null;
type ClosedWorkspaceView = { view: WorkspaceView; paneId: string; index: number };
type CurrentWorkspaceTarget = { pane: WorkspacePaneNode; view: WorkspaceView; session: SessionSummary };
type ScreenLockState = {
  reason: ScreenLockReason;
  lockedAt: number;
  mode: "preparing" | "vault" | "confirm" | "error";
  restoreVaultLocked: boolean | null;
  repairMarker: boolean;
  message: string;
} | null;
type HostKeyDecisionValue = "trust-once" | "append-to-profile" | "append-to-project" | "replace-for-profile";
type PortableVaultStatus = {
  exists: boolean;
  unlocked: boolean;
  path: string;
};
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

export default function App({ workspaceWindowId }: { workspaceWindowId?: string }) {
  const workspaceStorageKey = workspaceWindowId ? null : WORKSPACE_STORAGE_KEY;
  const workspacePanelStorageKey = workspaceWindowId ? null : WORKSPACE_PANEL_STORAGE_KEY;
  const ownerWindowId = workspaceWindowId ?? "main";
  const [initialWorkspace] = useState(() => loadWorkspaceSnapshot(workspaceStorageKey));
  const [terminalPrefs, setTerminalPrefs] = useState(loadTerminalPrefs);
  const [screenLock, setScreenLock] = useState<ScreenLockState>(() => loadInitialScreenLockState(terminalPrefs.requireMasterPassword));
  const [sessions, setSessionsState] = useState<SessionSummary[]>(emptySessions);
  const sessionsRef = useRef<SessionSummary[]>(sessions);
  const [logs, setLogs] = useState<Record<string, SessionEvent[]>>(emptyLogs);
  const [transfers, setTransfers] = useState<TransferTask[]>(emptyTransfers);
  const [dismissedTransferIds, setDismissedTransferIds] = useState<ReadonlySet<string>>(() => new Set());
  const [audit, setAudit] = useState<AuditRecord[]>(emptyAudit);
  const [grants, setGrants] = useState<McpGrant[]>(emptyGrants);
  const [mcpApprovals, setMcpApprovals] = useState<McpApprovalRequest[]>([]);
  const [hostKeys, setHostKeys] = useState<HostKeyStore>(emptyHostKeys);
  const [oneKeys, setOneKeys] = useState<OneKeySummary[]>([]);
  const [serialPorts, setSerialPorts] = useState<string[]>([]);
  const [serialCaptures, setSerialCaptures] = useState<Record<string, SerialCaptureFrame[]>>({});
  const [serialCaptureActionIds, setSerialCaptureActionIds] = useState<Set<string>>(() => new Set());
  const [serialControlBusyIds, setSerialControlBusyIds] = useState<Set<string>>(() => new Set());
  const [profileShortcutBusyIds, setProfileShortcutBusyIds] = useState<Set<string>>(() => new Set());
  const [terminalExportBusyViewIds, setTerminalExportBusyViewIds] = useState<Set<string>>(() => new Set());
  const [detachingWorkspaceViewIds, setDetachingWorkspaceViewIds] = useState<Set<string>>(() => new Set());
  const [disconnectingSessionIds, setDisconnectingSessionIds] = useState<Set<string>>(() => new Set());
  const [activeId, setActiveIdState] = useState(initialWorkspace.activeId);
  const activeIdRef = useRef(activeId);
  const [openMenu, setOpenMenu] = useState<string | null>(null);
  const [dialog, setDialog] = useState<SettingsDialog>(null);
  const [utilityDialog, setUtilityDialog] = useState<UtilityDialog>(null);
  const [keyManagerCredentialOperationToken, setKeyManagerCredentialOperationToken] = useState<number | null>(null);
  const [keyManagerCredentialSyncRevision, setKeyManagerCredentialSyncRevision] = useState(0);
  const [searchDialog, setSearchDialog] = useState<SearchDialogState>({ mode: "sessions", query: "" });
  const [draft, setDraft] = useState<SessionProfile>(() => createSessionDraft());
  const draftExpectedProfileRef = useRef<SessionProfile | null>(null);
  const [sendText, setSendText] = useState("");
  const [sendMode, setSendMode] = useState<SendMode>("text");
  const [sendCount, setSendCount] = useState(1);
  const [sendIntervalMs, setSendIntervalMs] = useState(1000);
  const [sendTarget, setSendTarget] = useState<SendTarget>("active");
  const [sendAdvancedOpen, setSendAdvancedOpen] = useState(false);
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
  const commandHistoryEntriesRef = useRef<CommandHistoryEntry[]>([]);
  const commandHistoryRevisionRef = useRef(0);
  const commandHistoryBackendReadyRef = useRef(false);
  const commandHistoryOperationRef = useRef<Promise<void>>(Promise.resolve());
  const pendingCommandHistoryRef = useRef<string[]>([]);
  const commandHistoryPolicyRef = useRef(commandHistoryPolicy);
  const commandHistoryEnabledRef = useRef(terminalPrefs.historyEnabled);
  const commandHistoryPersistedSettingsRef = useRef<{ enabled: boolean; limit: number; retentionDays: number } | null>(null);
  commandHistoryPolicyRef.current = commandHistoryPolicy;
  commandHistoryEnabledRef.current = terminalPrefs.historyEnabled;
  const commandHistory = commandHistoryEntries.map((entry) => entry.command);
  const [quickCommands, setQuickCommands] = useState<QuickCommand[]>(() => (
    normalizeQuickCommandLibrary(loadLocalValue<unknown>(QUICK_COMMAND_STORAGE_KEY, null)).items
  ));
  const [quickBarVisible, setQuickBarVisible] = useState(() => (
    loadLocalValue<unknown>(QUICK_BAR_VISIBLE_STORAGE_KEY, false) === true
  ));
  const [workspacePanels, setWorkspacePanels] = useState(() => {
    const stored = workspacePanelStorageKey
      ? loadLocalValue<unknown>(
        workspacePanelStorageKey,
        loadLocalValue<unknown>(LEGACY_WORKSPACE_PANEL_STORAGE_KEY, null),
      )
      : null;
    return normalizeWorkspacePanelVisibility(stored);
  });
  const [workspaceDockLayout, setWorkspaceDockLayout] = useState<WorkspaceDockLayout>(() => (
    normalizeWorkspaceDockLayout(workspacePanelStorageKey ? loadLocalValue<unknown>(workspacePanelStorageKey, null) : null)
  ));
  const [workspaceDockSizes, setWorkspaceDockSizes] = useState<WorkspaceDockSizes>(() => (
    normalizeWorkspaceDockSizes(workspacePanelStorageKey ? loadLocalValue<unknown>(workspacePanelStorageKey, null) : null)
  ));
  const [draggedWorkspacePanel, setDraggedWorkspacePanel] = useState<WorkspaceDockPanelId | null>(null);
  const [focusMode, setFocusMode] = useState(false);
  const [notice, setNotice] = useState<NoticeState>(null);
  const [hostKeyPrompt, setHostKeyPromptState] = useState<HostKeyPromptState | null>(null);
  const hostKeyPromptRef = useRef<HostKeyPromptState | null>(hostKeyPrompt);
  const [sessionSettingsSection, setSessionSettingsSection] = useState("会话");
  const [sessionSettingsMode, setSessionSettingsMode] = useState<SessionSettingsMode>("create");
  const [credentialPrompt, setCredentialPrompt] = useState<CredentialPromptState | null>(null);
  const [workspaceRoot, setWorkspaceRootState] = useState<WorkspaceNode | null>(initialWorkspace.root);
  const workspaceRootRef = useRef<WorkspaceNode | null>(workspaceRoot);
  const [activePaneId, setActivePaneIdState] = useState(initialWorkspace.activePaneId);
  const activePaneIdRef = useRef(activePaneId);
  const [zoomedPaneId, setZoomedPaneId] = useState("");
  const [workspaceGroupMove, setWorkspaceGroupMove] = useState<WorkspaceGroupMoveRequest>(null);
  const [workspaceViewRename, setWorkspaceViewRename] = useState<WorkspaceViewRenameRequest>(null);
  const [workspaceViewContextMenu, setWorkspaceViewContextMenu] = useState<WorkspaceViewContextMenuState>(null);
  const [closedWorkspaceViews, setClosedWorkspaceViewsState] = useState<ClosedWorkspaceView[]>([]);
  const closedWorkspaceViewsRef = useRef<ClosedWorkspaceView[]>(closedWorkspaceViews);
  const [workspaceKeymap, setWorkspaceKeymap] = useState<WorkspaceKeymap>(() => (
    normalizeWorkspaceKeymap(loadLocalValue<unknown>(
      WORKSPACE_KEYMAP_STORAGE_KEY,
      loadLocalValue<unknown>(LEGACY_WORKSPACE_KEYMAP_STORAGE_KEY, defaultWorkspaceKeymap),
    ))
  ));
  const [blockSelection, setBlockSelection] = useState(false);
  const [contextMenu, setContextMenu] = useState<ContextMenuState>(null);
  const [tabColors, setTabColorsState] = useState<Record<string, string>>(initialWorkspace.tabColors);
  const tabColorsRef = useRef(tabColors);
  const credentialResolverRef = useRef<{
    requestId: number;
    sessionId: string;
    resolve: (credentials: ConnectionCredentials | null) => void;
  } | null>(null);
  const credentialRequestIdRef = useRef(0);
  const startupAppliedRef = useRef(false);
  const syncInputDispatcherRef = useRef(new SyncInputDispatcher());
  const syncInputRef = useRef(false);
  const terminalInputEpochsRef = useRef(new Map<string, number>());
  const deletedTerminalInputSessionsRef = useRef(new Set<string>());
  const logSignatureRef = useRef<Record<string, string>>({});
  const activeLogRefreshGateRef = useRef(new KeyedRequestGate<string>());
  const sessionsSignatureRef = useRef("");
  const sessionSummaryRefreshGateRef = useRef(new KeyedRequestGate<"summaries">());
  const connectionAttemptGateRef = useRef(new KeyedRequestGate<string>());
  const connectionCloseGateRef = useRef(new KeyedRequestGate<string>());
  const startupHydrationGateRef = useRef(new KeyedRequestGate<StartupHydrationDomain>());
  const grantMutationGateRef = useRef(new KeyedRequestGate<"grants">());
  const hostKeyMutationGateRef = useRef(new KeyedRequestGate<"host-keys">());
  const hostKeyPromptOperationGateRef = useRef(new KeyedRequestGate<"scan" | "decision">());
  const keyManagerCredentialOperationGateRef = useRef(new KeyedRequestGate<"credentials">());
  const keyManagerProfileMutationGateRef = useRef(new KeyedRequestGate<string>());
  const sessionSettingsProfileMutationGateRef = useRef(new KeyedRequestGate<string>());
  const profileShortcutOperationGateRef = useRef(new KeyedRequestGate<string>());
  const oneKeyMutationGateRef = useRef(new KeyedRequestGate<"one-keys">());
  const serialCapturesRef = useRef<Record<string, SerialCaptureFrame[]>>({});
  const serialCaptureOperationGateRef = useRef(new KeyedRequestGate<string>());
  const serialCaptureActionTokensRef = useRef(new Map<string, number>());
  const serialControlOperationGateRef = useRef(new KeyedRequestGate<string>());
  const sendOperationGateRef = useRef(new KeyedRequestGate<"send">());
  const terminalExportOperationGateRef = useRef(new KeyedRequestGate<string>());
  const detachedWindowOperationGateRef = useRef(new KeyedRequestGate<string>());
  const serialAnalyzerWindowOperationGateRef = useRef(new KeyedRequestGate<string>());
  const pendingProfileDeletionRef = useRef(new Map<string, { token: number; profileName: string }>());
  const resolvedMcpApprovalsRef = useRef(new Set<string>());
  const pendingMcpApprovalsRef = useRef(new Set<string>());
  const screenLockOperationGateRef = useRef(new KeyedRequestGate<"prepare" | "unlock">());
  const detachedCommandHandlerRef = useRef<(command: DetachedPaneCommand) => DetachedPaneResult | null>(() => null);
  const profileUpdateHandlerRef = useRef<(summary: SessionSummary) => void>(() => {});
  const profileDeleteHandlerRef = useRef<(payload: DeleteSessionProfileResponse | string) => void>(() => {});
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
    terminalExportBusy: terminalExportBusyViewIds.has(activeWorkspaceView?.id ?? ""),
  };
  syncInputRef.current = syncInput;
  screenLockRef.current = screenLock;
  sessionsRef.current = sessions;
  activeIdRef.current = activeId;
  tabColorsRef.current = tabColors;
  workspaceRootRef.current = workspaceRoot;
  activePaneIdRef.current = activePaneId;
  closedWorkspaceViewsRef.current = closedWorkspaceViews;
  hostKeyPromptRef.current = hostKeyPrompt;
  detachedCommandHandlerRef.current = (command) => {
    if (command.action === "lock-screen") {
      lockScreen("manual");
    } else if (command.action === "connect") {
      void connectSession(command.sessionId, undefined, false);
    } else if (command.action === "disconnect") {
      void disconnectSession(command.sessionId, false);
    } else {
      return reattachDetachedPane(command);
    }
    return null;
  };

  function setWorkspaceRoot(update: SetStateAction<WorkspaceNode | null>) {
    const next = typeof update === "function" ? update(workspaceRootRef.current) : update;
    workspaceRootRef.current = next;
    setWorkspaceRootState(next);
  }

  function setSessions(update: SetStateAction<SessionSummary[]>) {
    const next = typeof update === "function" ? update(sessionsRef.current) : update;
    sessionsRef.current = next;
    setSessionsState(next);
  }

  function setActiveId(update: SetStateAction<string>) {
    const next = typeof update === "function" ? update(activeIdRef.current) : update;
    activeIdRef.current = next;
    setActiveIdState(next);
  }

  function setTabColors(update: SetStateAction<Record<string, string>>) {
    const next = typeof update === "function" ? update(tabColorsRef.current) : update;
    tabColorsRef.current = next;
    setTabColorsState(next);
  }

  function setActivePaneId(update: SetStateAction<string>) {
    const next = typeof update === "function" ? update(activePaneIdRef.current) : update;
    activePaneIdRef.current = next;
    setActivePaneIdState(next);
  }

  function setClosedWorkspaceViews(update: SetStateAction<ClosedWorkspaceView[]>) {
    const next = typeof update === "function" ? update(closedWorkspaceViewsRef.current) : update;
    closedWorkspaceViewsRef.current = next;
    setClosedWorkspaceViewsState(next);
  }

  function setHostKeyPrompt(update: SetStateAction<HostKeyPromptState | null>) {
    const next = typeof update === "function" ? update(hostKeyPromptRef.current) : update;
    hostKeyPromptRef.current = next;
    setHostKeyPromptState(next);
  }
  profileUpdateHandlerRef.current = (summary) => {
    if (!summary?.profile?.id) return;
    const prompt = hostKeyPromptRef.current;
    if (prompt?.profile.id === summary.profile.id
      && !hostKeyProfileSnapshotMatches(prompt.profile, prepareSessionProfile(summary.profile))) {
      closeHostKeyPrompt();
    }
    applySavedSessionState(summary, false);
  };
  profileDeleteHandlerRef.current = (payload) => {
    const sessionId = typeof payload === "string" ? payload : payload?.deletedProfileId;
    if (!sessionId) return;
    const response = typeof payload === "string"
      ? deleteSessionProfileFromClientState(sessionId, { sessions: sessionsRef.current, oneKeys, hostKeys, grants })
      : payload;
    const pending = pendingProfileDeletionRef.current.get(sessionId);
    if (pending) pendingProfileDeletionRef.current.delete(sessionId);
    applyDeletedSessionProfile(response);
    void refresh();
    if (pending) {
      setNotice({
        title: "会话已删除",
        message: `已删除 ${pending.profileName}；磁盘日志仍可在日志管理器中查看或清理。`,
      });
    }
  };

  function updateSyncInput(enabled: boolean) {
    if (!enabled && syncInputRef.current) {
      syncInputDispatcherRef.current.cancelBroadcasts();
    }
    syncInputRef.current = enabled;
    setSyncInput(enabled);
  }

  function captureTerminalInputEpoch(sessionId: string): number | null {
    if (deletedTerminalInputSessionsRef.current.has(sessionId)) return null;
    return terminalInputEpochsRef.current.get(sessionId) ?? 0;
  }

  function terminalInputIsCurrent(sessionId: string, epoch: number) {
    return !deletedTerminalInputSessionsRef.current.has(sessionId)
      && (terminalInputEpochsRef.current.get(sessionId) ?? 0) === epoch;
  }

  function invalidateTerminalInputSession(sessionId: string) {
    deletedTerminalInputSessionsRef.current.add(sessionId);
    terminalInputEpochsRef.current.set(sessionId, (terminalInputEpochsRef.current.get(sessionId) ?? 0) + 1);
  }

  function restoreTerminalInputSession(sessionId: string) {
    deletedTerminalInputSessionsRef.current.delete(sessionId);
  }

  function restoreTerminalInputSessions(nextSessions: readonly SessionSummary[]) {
    for (const session of nextSessions) restoreTerminalInputSession(session.profile.id);
  }

  function updateTransfers(update: SetStateAction<TransferTask[]>) {
    startupHydrationGateRef.current.invalidate("transfers");
    setTransfers(update);
  }

  function applyCommandHistorySnapshot(snapshot: CommandHistorySnapshot, committedCommand?: string) {
    if (!Number.isSafeInteger(snapshot.revision) || snapshot.revision < commandHistoryRevisionRef.current) return;
    commandHistoryRevisionRef.current = snapshot.revision;
    if (committedCommand !== undefined) {
      const index = pendingCommandHistoryRef.current.indexOf(committedCommand);
      if (index >= 0) pendingCommandHistoryRef.current.splice(index, 1);
    }
    if (!commandHistoryEnabledRef.current) return;
    void import("./command-history-state").then(({ normalizeCommandHistory, recordCommandHistory }) => {
      if (snapshot.revision < commandHistoryRevisionRef.current) return;
      let entries = normalizeCommandHistory(
        { version: 2, entries: Array.isArray(snapshot.entries) ? snapshot.entries : [] },
        commandHistoryPolicyRef.current,
      );
      for (const command of pendingCommandHistoryRef.current) {
        entries = recordCommandHistory(entries, command, commandHistoryPolicyRef.current);
      }
      commandHistoryEntriesRef.current = entries;
      setCommandHistoryEntries(entries);
    });
  }

  function enqueueCommandHistoryOperation(
    operation: () => Promise<CommandHistorySnapshot>,
    committedCommand?: string,
  ) {
    commandHistoryOperationRef.current = commandHistoryOperationRef.current.then(async () => {
      try {
        applyCommandHistorySnapshot(await operation(), committedCommand);
      } catch (error) {
        console.warn("PortMate command history persistence failed", error);
      }
    });
  }

  const dismissTransfer = useCallback((transferId: string) => {
    setDismissedTransferIds((current) => addDismissedTransferId(current, transferId));
  }, []);

  function updateAudit(next: AuditRecord[]) {
    startupHydrationGateRef.current.invalidate("audit");
    setAudit(next);
  }

  function updateGrants(next: McpGrant[]) {
    grantMutationGateRef.current.invalidate("grants");
    startupHydrationGateRef.current.invalidate("grants");
    setGrants(next);
  }

  function beginGrantMutation() {
    return grantMutationGateRef.current.replace("grants");
  }

  function commitGrantMutation(next: McpGrant[], token: number) {
    if (!grantMutationGateRef.current.isCurrent("grants", token)) return false;
    updateGrants(next);
    return true;
  }

  function finishGrantMutation(token: number) {
    grantMutationGateRef.current.finish("grants", token);
  }

  function updateHostKeys(next: HostKeyStore) {
    hostKeyMutationGateRef.current.invalidate("host-keys");
    startupHydrationGateRef.current.invalidate("host-keys");
    setHostKeys(next);
  }

  function beginHostKeyMutation() {
    return hostKeyMutationGateRef.current.replace("host-keys");
  }

  function commitHostKeyMutation(next: HostKeyStore, token: number) {
    if (!hostKeyMutationGateRef.current.isCurrent("host-keys", token)) return false;
    updateHostKeys(next);
    return true;
  }

  function finishHostKeyMutation(token: number) {
    hostKeyMutationGateRef.current.finish("host-keys", token);
  }

  function beginKeyManagerProfileMutation(profileId: string) {
    return keyManagerProfileMutationGateRef.current.replace(profileId);
  }

  function commitKeyManagerProfileMutation(saved: SessionSummary, token: number, activateWorkspace = true) {
    const profileId = saved.profile.id;
    if (!keyManagerProfileMutationGateRef.current.isCurrent(profileId, token)) return false;
    applySavedSessionState(saved, activateWorkspace);
    return true;
  }

  function isKeyManagerProfileMutationCurrent(profileId: string, token: number) {
    return keyManagerProfileMutationGateRef.current.isCurrent(profileId, token);
  }

  function finishKeyManagerProfileMutation(profileId: string, token: number, committed: boolean) {
    const gate = keyManagerProfileMutationGateRef.current;
    if (!committed && gate.isCurrent(profileId, token)) void refreshSessionSummaries();
    gate.finish(profileId, token);
  }

  function beginProfileShortcutOperation(profileId: string) {
    const token = profileShortcutOperationGateRef.current.begin(profileId);
    if (token !== null) {
      setProfileShortcutBusyIds((current) => new Set(current).add(profileId));
    }
    return token;
  }

  function finishProfileShortcutOperation(profileId: string, token: number) {
    if (!profileShortcutOperationGateRef.current.finish(profileId, token)) return;
    setProfileShortcutBusyIds((current) => {
      if (!current.has(profileId)) return current;
      const next = new Set(current);
      next.delete(profileId);
      return next;
    });
  }

  function invalidateProfileShortcutOperation(profileId: string) {
    profileShortcutOperationGateRef.current.invalidate(profileId);
    setProfileShortcutBusyIds((current) => {
      if (!current.has(profileId)) return current;
      const next = new Set(current);
      next.delete(profileId);
      return next;
    });
  }

  function beginTerminalExportOperation(viewId: string) {
    const token = terminalExportOperationGateRef.current.begin(viewId);
    if (token !== null) {
      setTerminalExportBusyViewIds((current) => new Set(current).add(viewId));
    }
    return token;
  }

  function finishTerminalExportOperation(viewId: string, token: number) {
    if (!terminalExportOperationGateRef.current.finish(viewId, token)) return;
    setTerminalExportBusyViewIds((current) => {
      if (!current.has(viewId)) return current;
      const next = new Set(current);
      next.delete(viewId);
      return next;
    });
  }

  function invalidateTerminalExportOperation(viewId: string) {
    terminalExportOperationGateRef.current.invalidate(viewId);
    setTerminalExportBusyViewIds((current) => {
      if (!current.has(viewId)) return current;
      const next = new Set(current);
      next.delete(viewId);
      return next;
    });
  }

  function invalidateTerminalExportsForSession(sessionId: string) {
    for (const pane of workspacePaneLeaves(workspaceRootRef.current)) {
      for (const view of pane.views) {
        if (view.sessionId === sessionId) invalidateTerminalExportOperation(view.id);
      }
    }
  }

  function invalidateAllTerminalExportOperations() {
    terminalExportOperationGateRef.current.invalidateAll();
    setTerminalExportBusyViewIds((current) => current.size ? new Set() : current);
  }

  function beginKeyManagerCredentialOperation() {
    const token = keyManagerCredentialOperationGateRef.current.begin("credentials");
    if (token !== null) setKeyManagerCredentialOperationToken(token);
    return token;
  }

  function finishKeyManagerCredentialOperation(token: number, changed = true) {
    if (!keyManagerCredentialOperationGateRef.current.finish("credentials", token)) return;
    setKeyManagerCredentialOperationToken((current) => current === token ? null : current);
    if (changed) setKeyManagerCredentialSyncRevision((current) => current + 1);
  }

  function updateOneKeys(next: OneKeySummary[]) {
    oneKeyMutationGateRef.current.invalidate("one-keys");
    startupHydrationGateRef.current.invalidate("one-keys");
    setOneKeys(next);
  }

  function beginOneKeyMutation() {
    return oneKeyMutationGateRef.current.replace("one-keys");
  }

  function commitOneKeyMutation(next: OneKeySummary[], token: number) {
    if (!oneKeyMutationGateRef.current.isCurrent("one-keys", token)) return false;
    updateOneKeys(next);
    return true;
  }

  function finishOneKeyMutation(token: number) {
    oneKeyMutationGateRef.current.finish("one-keys", token);
  }

  function beginSessionDisconnect(sessionId: string) {
    const token = connectionCloseGateRef.current.begin(sessionId);
    if (token !== null) {
      setDisconnectingSessionIds((current) => new Set(current).add(sessionId));
    }
    return token;
  }

  function finishSessionDisconnect(sessionId: string, token: number) {
    if (!connectionCloseGateRef.current.finish(sessionId, token)) return;
    setDisconnectingSessionIds((current) => {
      if (!current.has(sessionId)) return current;
      const next = new Set(current);
      next.delete(sessionId);
      return next;
    });
  }

  function commitScreenLock(next: ScreenLockState) {
    screenLockRef.current = next;
    setScreenLock(next);
  }

  function clearScreenLock() {
    screenLockOperationGateRef.current.invalidateAll();
    try {
      window.localStorage.removeItem(SCREEN_LOCK_STORAGE_KEY);
    } catch {
      // The in-memory lock still clears when local storage is unavailable.
    }
    clearScreenLockVaultRestoreState();
    commitScreenLock(null);
  }

  async function prepareScreenLock(state: NonNullable<ScreenLockState>, replaceExisting = false) {
    const gate = screenLockOperationGateRef.current;
    gate.invalidate("unlock");
    const token = replaceExisting ? gate.replace("prepare") : gate.begin("prepare");
    if (token === null) return;
    const isCurrent = () => (
      gate.isCurrent("prepare", token)
        && screenLockRef.current?.lockedAt === state.lockedAt
    );
    const preparing = { ...state, mode: "preparing" as const, message: "" };
    let restoreVaultLocked = state.restoreVaultLocked;
    commitScreenLock(preparing);
    try {
      if (!isBackendAvailable()) {
        if (isCurrent()) {
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
        if (!isCurrent()) return;
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
        if (!isCurrent()) return;
        commitScreenLock({
          ...preparing,
          mode: "vault",
          restoreVaultLocked,
          message: "Portable Vault 已锁定",
        });
      } catch {
        if (!isCurrent()) return;
        commitScreenLock({
          ...preparing,
          mode: "error",
          restoreVaultLocked,
          message: "无法确认 Portable Vault 状态",
        });
      }
    } finally {
      gate.finish("prepare", token);
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
    void prepareScreenLock(next, true);
  }

  async function unlockScreen(password = "") {
    const gate = screenLockOperationGateRef.current;
    const token = gate.begin("unlock");
    if (token === null) return;
    const current = screenLockRef.current;
    try {
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
      if (!gate.isCurrent("unlock", token) || screenLockRef.current?.lockedAt !== current.lockedAt) {
        if (screenLockRef.current) {
          try {
            await invokeBackend<PortableVaultStatus>("lock_portable_vault", {});
          } catch {
            // A newer lock generation owns the visible recovery state; remain fail-closed when possible.
          }
        }
        return;
      }
      if (current.restoreVaultLocked) {
        try {
          await invokeBackend<PortableVaultStatus>("lock_portable_vault", {});
        } catch {
          throw new Error("凭据锁定状态恢复失败，请重试");
        }
      }
      if (gate.isCurrent("unlock", token) && screenLockRef.current?.lockedAt === current.lockedAt) {
        clearScreenLock();
      }
    } finally {
      gate.finish("unlock", token);
    }
  }

  function retryPrepareScreenLock() {
    const current = screenLockRef.current;
    if (current?.mode === "error") void prepareScreenLock(current);
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
      void prepareScreenLock(current, true);
    }
  }, []);

  useEffect(() => {
    const refreshScreenLock = () => {
      const current = screenLockRef.current;
      const next = readStoredScreenLockState(current?.lockedAt ?? Date.now());
      if (next === undefined) return;
      if (!next) {
        if (current) clearScreenLock();
        return;
      }
      if (current?.lockedAt === next.lockedAt) return;
      clearScreenLockVaultRestoreState();
      commitScreenLock(next);
      void prepareScreenLock(next, true);
    };
    const handleStorage = (event: StorageEvent) => {
      if (event.key === SCREEN_LOCK_STORAGE_KEY || event.key === null) refreshScreenLock();
    };
    const timer = window.setInterval(refreshScreenLock, 500);
    window.addEventListener("storage", handleStorage);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("storage", handleStorage);
    };
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
    if (workspaceWindowId || startupAppliedRef.current || !sessions.length) return;
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
        if (session && sessionConnectionAction(session.runtime.status) === "connect") {
          await connectSession(sessionId, undefined, true, "silent");
        }
      }
    })();
  }, [sessions, workspaceWindowId]);

  useEffect(() => {
    if (!workspaceStorageKey) return;
    saveLocalValue<WorkspaceSnapshot>(workspaceStorageKey, {
      version: 4,
      root: workspaceRoot,
      activePaneId,
      activeId,
      tabColors,
    });
  }, [workspaceStorageKey, workspaceRoot, activePaneId, activeId, tabColors]);

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
      const panes = workspacePaneLeaves(workspaceRootRef.current);
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
        { terminalKeyMode: currentWorkspaceTarget()?.view.keyMode ?? activeTerminalKeyMode },
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
        const nextPane = findWorkspacePaneInDirection(
          workspaceRootRef.current,
          activePaneIdRef.current,
          hotkey.direction,
        );
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
    let stopListening: (() => void) | undefined;
    void import("./command-history-state").then(async ({ normalizeCommandHistory }) => {
      if (disposed) return;
      const localEntries = normalizeCommandHistory(
        loadLocalValue<unknown>(COMMAND_HISTORY_STORAGE_KEY, null),
        commandHistoryPolicyRef.current,
      );
      if (!isBackendAvailable()) {
        const entries = commandHistoryEnabledRef.current ? localEntries : [];
        commandHistoryEntriesRef.current = entries;
        setCommandHistoryEntries(entries);
        setCommandHistoryReady(true);
        return;
      }
      try {
        stopListening = await listen<CommandHistorySnapshot>(COMMAND_HISTORY_UPDATED_EVENT, (event) => {
          if (!disposed) applyCommandHistorySnapshot(event.payload);
        });
        if (disposed) {
          stopListening();
          return;
        }
      } catch (error) {
        console.warn("PortMate command history live synchronization failed", error);
      }
      try {
        const initial = await invokeBackend<CommandHistorySnapshot>("list_command_history", {
          limit: commandHistoryPolicyRef.current.limit,
          retentionDays: commandHistoryPolicyRef.current.retentionDays,
        });
        if (disposed) return;
        const snapshot = initial.migrated
          ? await invokeBackend<CommandHistorySnapshot>("merge_command_history", {
            entries: commandHistoryEnabledRef.current ? localEntries : [],
            limit: commandHistoryPolicyRef.current.limit,
            retentionDays: commandHistoryPolicyRef.current.retentionDays,
          })
          : await invokeBackend<CommandHistorySnapshot>("migrate_command_history", {
            entries: commandHistoryEnabledRef.current ? localEntries : [],
            limit: commandHistoryPolicyRef.current.limit,
            retentionDays: commandHistoryPolicyRef.current.retentionDays,
          });
        if (disposed) return;
        const enabledSnapshot = !commandHistoryEnabledRef.current && snapshot.entries.length
          ? await invokeBackend<CommandHistorySnapshot>("clear_command_history", {})
          : snapshot;
        if (disposed) return;
        commandHistoryBackendReadyRef.current = true;
        applyCommandHistorySnapshot(enabledSnapshot);
        for (const command of [...pendingCommandHistoryRef.current]) {
          enqueueCommandHistoryOperation(
            () => invokeBackend<CommandHistorySnapshot>("record_command_history", {
              command,
              limit: commandHistoryPolicyRef.current.limit,
              retentionDays: commandHistoryPolicyRef.current.retentionDays,
            }),
            command,
          );
        }
      } catch (error) {
        if (disposed) return;
        console.warn("PortMate command history initialization failed", error);
        const entries = commandHistoryEnabledRef.current ? localEntries : [];
        commandHistoryEntriesRef.current = entries;
        setCommandHistoryEntries(entries);
      } finally {
        if (!disposed) setCommandHistoryReady(true);
      }
    });
    return () => {
      disposed = true;
      stopListening?.();
    };
  }, []);

  useEffect(() => {
    if (!isBackendAvailable()) return;
    let disposed = false;
    const unlisten = new Set<() => void>();
    void listen<SessionSummary>(SESSION_PROFILE_UPDATED_EVENT, (event) => {
      if (!disposed) profileUpdateHandlerRef.current(event.payload);
    }).then((nextUnlisten) => {
      if (disposed) nextUnlisten();
      else unlisten.add(nextUnlisten);
    }).catch(() => {});
    void listen<DeleteSessionProfileResponse | string>(SESSION_PROFILE_DELETED_EVENT, (event) => {
      if (!disposed) profileDeleteHandlerRef.current(event.payload);
    }).then((nextUnlisten) => {
      if (disposed) nextUnlisten();
      else unlisten.add(nextUnlisten);
    }).catch(() => {});
    return () => {
      disposed = true;
      for (const stopListening of unlisten) stopListening();
      unlisten.clear();
    };
  }, []);

  useEffect(() => {
    if (!isBackendAvailable()) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listenTerminalByteEvents()
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
    if (!commandHistoryReady) return;
    let disposed = false;
    void import("./command-history-state").then((history) => {
      if (disposed) return;
      pendingCommandHistoryRef.current = terminalPrefs.historyEnabled
        ? history.normalizePendingCommandHistory(
          pendingCommandHistoryRef.current,
          commandHistoryPolicy,
        )
        : [];
      const normalized = history.normalizeCommandHistory(
        history.commandHistorySnapshot(commandHistoryEntries),
        commandHistoryPolicy,
      );
      if (!history.commandHistoryEntriesEqual(commandHistoryEntries, normalized)) {
        commandHistoryEntriesRef.current = normalized;
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

      if (!commandHistoryBackendReadyRef.current) return;
      const settings = {
        enabled: terminalPrefs.historyEnabled,
        limit: commandHistoryPolicy.limit,
        retentionDays: commandHistoryPolicy.retentionDays,
      };
      const previous = commandHistoryPersistedSettingsRef.current;
      commandHistoryPersistedSettingsRef.current = settings;
      if (!previous) return;
      if (!settings.enabled) {
        if (previous.enabled) {
          enqueueCommandHistoryOperation(() => invokeBackend<CommandHistorySnapshot>("clear_command_history", {}));
        }
      } else if (!previous.enabled) {
        enqueueCommandHistoryOperation(() => invokeBackend<CommandHistorySnapshot>("merge_command_history", {
          entries: normalized,
          limit: settings.limit,
          retentionDays: settings.retentionDays,
        }));
      } else if (previous.limit !== settings.limit || previous.retentionDays !== settings.retentionDays) {
        enqueueCommandHistoryOperation(() => invokeBackend<CommandHistorySnapshot>("normalize_command_history", {
          limit: settings.limit,
          retentionDays: settings.retentionDays,
        }));
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
    if (!workspacePanelStorageKey) return;
    saveLocalValue(workspacePanelStorageKey, {
      version: 7,
      panels: workspacePanels,
      docks: workspaceDockLayout,
      sizes: workspaceDockSizes,
    });
  }, [workspacePanelStorageKey, workspaceDockLayout, workspaceDockSizes, workspacePanels]);

  useEffect(() => {
    const preventNativeContextMenu = (event: MouseEvent) => {
      event.preventDefault();
    };
    window.addEventListener("contextmenu", preventNativeContextMenu, { capture: true });
    return () => window.removeEventListener("contextmenu", preventNativeContextMenu, { capture: true });
  }, []);

  useEffect(() => {
    const dismissInteractionLayer = () => {
      setOpenMenu(null);
      setContextMenu(null);
      setWorkspaceViewContextMenu(null);
    };
    window.addEventListener(INTERACTION_LAYER_DISMISS_EVENT, dismissInteractionLayer);
    return () => window.removeEventListener(INTERACTION_LAYER_DISMISS_EVENT, dismissInteractionLayer);
  }, []);

  useEffect(() => {
    if (!contextMenu) return;
    const closeOnScroll = (event: Event) => {
      // XTerm can emit a deferred scroll after selection/search work. Keep that
      // bookkeeping event from immediately dismissing a newly opened menu;
      // actual terminal wheel input is handled separately below.
      if (event.target instanceof Element && event.target.closest(".terminal-canvas")) return;
      setContextMenu(null);
    };
    const closeOnWheel = () => setContextMenu(null);
    const closeOnResize = () => setContextMenu(null);
    window.addEventListener("scroll", closeOnScroll, true);
    window.addEventListener("wheel", closeOnWheel, true);
    window.addEventListener("resize", closeOnResize);
    return () => {
      window.removeEventListener("scroll", closeOnScroll, true);
      window.removeEventListener("wheel", closeOnWheel, true);
      window.removeEventListener("resize", closeOnResize);
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
      updateTransfers((current) => mergeTransfers(current, event.payload));
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
      setMcpApprovals((current) => mergeMcpApprovals(
        current,
        [],
        Date.now(),
        resolvedMcpApprovalsRef.current,
        pendingMcpApprovalsRef.current,
      ));
    }, 1000);
    void listen<unknown>(MCP_APPROVAL_EVENT, (event) => {
      if (disposed) return;
      setMcpApprovals((current) => mergeMcpApprovals(
        current,
        [event.payload],
        Date.now(),
        resolvedMcpApprovalsRef.current,
        pendingMcpApprovalsRef.current,
      ));
    }).then(async (nextUnlisten) => {
      if (disposed) {
        nextUnlisten();
        return;
      }
      unlisten = nextUnlisten;
      try {
        const pending = await invokeBackend<unknown[]>("list_mcp_approvals", {});
        if (!disposed) setMcpApprovals((current) => mergeMcpApprovals(
          current,
          pending,
          Date.now(),
          resolvedMcpApprovalsRef.current,
          pendingMcpApprovalsRef.current,
        ));
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
        link: effect.kind === "custom-link" ? effect.value : undefined,
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
      if (!message) return;
      const result = detachedCommandHandlerRef.current(message.payload);
      const source = event.source as WindowProxy | null;
      if (result && source && typeof source.closed === "boolean") {
        source.postMessage({
          type: DETACHED_PANE_RESULT_MESSAGE_TYPE,
          payload: result,
        }, event.origin);
      }
    };
    window.addEventListener("message", handleBrowserMessage);
    if (isBackendAvailable()) {
      void listen<unknown>(DETACHED_PANE_EVENT, (event) => {
        const command = normalizeDetachedPaneCommand(event.payload);
        if (!command) return;
        const result = detachedCommandHandlerRef.current(command);
        if (result) void emitTo(command.windowId, DETACHED_PANE_RESULT_EVENT, result).catch(() => {});
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
    if (resolvedMcpApprovalsRef.current.has(approvalId) || pendingMcpApprovalsRef.current.has(approvalId)) return;
    pendingMcpApprovalsRef.current.add(approvalId);
    try {
      await invokeBackend<void>("respond_mcp_approval", { approvalId, approved });
      rememberResolvedMcpApproval(resolvedMcpApprovalsRef.current, approvalId);
      setMcpApprovals((current) => mergeMcpApprovals(
        current.filter((request) => request.id !== approvalId),
        [],
        Date.now(),
        resolvedMcpApprovalsRef.current,
        pendingMcpApprovalsRef.current,
      ));
    } finally {
      pendingMcpApprovalsRef.current.delete(approvalId);
    }
  }

  function expireMcpApproval(approvalId: string) {
    rememberResolvedMcpApproval(resolvedMcpApprovalsRef.current, approvalId);
    setMcpApprovals((current) => mergeMcpApprovals(
      current.filter((request) => request.id !== approvalId),
      [],
      Date.now(),
      resolvedMcpApprovalsRef.current,
      pendingMcpApprovalsRef.current,
    ));
  }

  async function refresh() {
    const gate = sessionSummaryRefreshGateRef.current;
    const token = gate.replace("summaries");
    const supplementalHydration = Promise.all([
      hydrateStartupValue("transfers", () => readStartupValue("list_transfers", emptyTransfers), setTransfers, true),
      hydrateStartupValue("audit", () => readStartupValue("list_mcp_audit", emptyAudit), setAudit, true),
      hydrateStartupValue("grants", () => readStartupValue("list_mcp_grants", emptyGrants), setGrants, true),
      hydrateStartupValue("host-keys", () => readStartupValue("list_host_keys", emptyHostKeys), setHostKeys, true),
      hydrateStartupValue("one-keys", () => readStartupValue("list_one_keys", []), setOneKeys, true),
      hydrateStartupValue("serial-ports", () => readStartupValue("list_serial_ports", []), setSerialPorts, true),
    ]);
    try {
      const nextSessions = await callBackend("list_sessions", {}, emptySessions);
      if (gate.isCurrent("summaries", token)) {
        restoreTerminalInputSessions(nextSessions);
        sessionsSignatureRef.current = sessionsSignature(nextSessions);
        const snapshot = cloneSessionSummaries(nextSessions);
        setSessions(snapshot);
        const restored = reconcileWorkspaceSnapshot({
          version: 4,
          root: workspaceRootRef.current,
          activePaneId: activePaneIdRef.current,
          activeId: activeIdRef.current,
          tabColors: tabColorsRef.current,
        }, nextSessions.map((session) => session.profile.id), {
          fallbackToFirst: !workspaceWindowId,
        });
        setWorkspaceRoot(restored.root);
        setActivePaneId(restored.activePaneId);
        setActiveId(restored.activeId);
        setTabColors(restored.tabColors);
      } else {
        void refreshSessionSummaries();
      }

      await supplementalHydration;

      if (!gate.isCurrent("summaries", token)) return;
      for (const session of nextSessions) {
        if (!gate.isCurrent("summaries", token)) return;
        await refreshActiveLog(session.profile.id, 160);
      }
    } finally {
      gate.finish("summaries", token);
    }
  }

  async function hydrateStartupValue<T>(
    domain: StartupHydrationDomain,
    load: () => Promise<T>,
    apply: (value: T) => void,
    replace = false,
  ) {
    const gate = startupHydrationGateRef.current;
    const token = replace ? gate.replace(domain) : gate.begin(domain);
    if (token === null) return;
    try {
      const value = await load();
      if (!gate.isCurrent(domain, token)) {
        void hydrateStartupValue(domain, load, apply);
        return;
      }
      apply(value);
    } catch {
      if (!gate.isCurrent(domain, token)) void hydrateStartupValue(domain, load, apply);
    } finally {
      gate.finish(domain, token);
    }
  }

  function readStartupValue<T>(command: string, fallback: T): Promise<T> {
    return isBackendAvailable() ? invokeBackend<T>(command, {}) : Promise.resolve(fallback);
  }

  async function refreshActiveLog(sessionId: string, limit = 600) {
    if (!sessionsRef.current.some((session) => session.profile.id === sessionId)) return;
    const gate = activeLogRefreshGateRef.current;
    const token = gate.begin(sessionId);
    if (token === null) return;
    try {
      const nextLog = await invokeBackend<SessionEvent[]>("tail_log", { sessionId, limit });
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

  function beginSerialCaptureAction(sessionId: string): number | null {
    const session = sessionsRef.current.find((candidate) => candidate.profile.id === sessionId);
    if (!session || session.profile.connection.kind !== "serial") return null;
    if (serialCaptureActionTokensRef.current.has(sessionId)) return null;
    const token = serialCaptureOperationGateRef.current.replace(sessionId);
    serialCaptureActionTokensRef.current.set(sessionId, token);
    setSerialCaptureActionIds((current) => {
      if (current.has(sessionId)) return current;
      return new Set(current).add(sessionId);
    });
    return token;
  }

  function finishSerialCaptureAction(sessionId: string, token: number) {
    if (serialCaptureActionTokensRef.current.get(sessionId) !== token) return;
    serialCaptureActionTokensRef.current.delete(sessionId);
    serialCaptureOperationGateRef.current.finish(sessionId, token);
    setSerialCaptureActionIds((current) => {
      if (!current.has(sessionId)) return current;
      const next = new Set(current);
      next.delete(sessionId);
      return next;
    });
  }

  async function refreshSerialCapture(sessionId: string) {
    const session = sessionsRef.current.find((candidate) => candidate.profile.id === sessionId);
    if (!session || session.profile.connection.kind !== "serial") return;
    const gate = serialCaptureOperationGateRef.current;
    const token = gate.begin(sessionId);
    if (token === null) return;
    try {
      const current = serialCapturesRef.current[sessionId] ?? [];
      const snapshot = await invokeBackend<SerialCaptureSnapshot>("list_serial_capture", {
        sessionId,
        afterId: current.at(-1)?.id ?? null,
      });
      if (!gate.isCurrent(sessionId, token)) return;
      const next = mergeSerialCaptureSnapshot(current, snapshot);
      if (next !== current) storeSerialCapture(sessionId, next);
    } catch {
      // Capture polling is best-effort; transport status and terminal output remain authoritative.
    } finally {
      gate.finish(sessionId, token);
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
    const token = beginSerialCaptureAction(sessionId);
    if (token === null) return;
    const gate = serialCaptureOperationGateRef.current;
    try {
      if (!isBackendAvailable()) {
        storeSerialCapture(sessionId, []);
        return;
      }
      const snapshot = await invokeBackend<SerialCaptureSnapshot>("clear_serial_capture", { sessionId });
      if (!gate.isCurrent(sessionId, token)) return;
      storeSerialCapture(sessionId, mergeSerialCaptureSnapshot([], snapshot));
    } catch (error) {
      if (gate.isCurrent(sessionId, token)) {
        setNotice({ title: "清空串口捕获失败", message: formatError(error) });
      }
    } finally {
      finishSerialCaptureAction(sessionId, token);
    }
  }

  async function exportSerialCapture(sessionId: string, frameIds: string[]) {
    const token = beginSerialCaptureAction(sessionId);
    if (token === null) return;
    const gate = serialCaptureOperationGateRef.current;
    try {
      const result = await invokeBackend<ExportSerialCaptureResult>("export_serial_capture", {
        request: { sessionId, frameIds },
      });
      if (!gate.isCurrent(sessionId, token)) return;
      setNotice({
        title: "串口捕获已导出",
        message: `${result.frames} 帧 · ${formatBytes(result.capturedBytes)} · ${result.path}\nSHA-256 ${result.sha256}`,
      });
    } catch (error) {
      if (gate.isCurrent(sessionId, token)) {
        setNotice({ title: "导出串口捕获失败", message: formatError(error) });
      }
    } finally {
      finishSerialCaptureAction(sessionId, token);
    }
  }

  async function refreshSessionSummaries() {
    const gate = sessionSummaryRefreshGateRef.current;
    const token = gate.begin("summaries");
    if (token === null) return;
    try {
      const nextSessions = await invokeBackend<SessionSummary[]>("list_sessions", {});
      if (!gate.isCurrent("summaries", token)) return;
      restoreTerminalInputSessions(nextSessions);
      const signature = sessionsSignature(nextSessions);
      if (sessionsSignatureRef.current === signature) return;
      sessionsSignatureRef.current = signature;
      const snapshot = cloneSessionSummaries(nextSessions);
      setSessions(snapshot);
      saveLocalSessionSummaries(snapshot);
    } catch {
      // Polling failures retain the last valid session snapshot.
    } finally {
      gate.finish("summaries", token);
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
    const renderedActiveId = active?.profile.id ?? "";
    const activeSnapshotStale = renderedActiveId !== activeIdRef.current;
    const currentActive = active?.profile.id === activeIdRef.current
      ? sessionsRef.current.find((session) => session.profile.id === activeIdRef.current)
      : undefined;
    const terminalKeyMode = terminalKeyModeMenuItems[item];
    const sessionBoundItem = terminalKeyMode || [
      "查找",
      "自由输入",
      "跳转到行",
      "导出终端文本",
      "导出选中文本",
      "启动会话",
      "关闭会话",
      "会话设置",
      "端口转发",
      "触发器",
      "Sysmon",
      "串口分析器",
      "Tmux",
      "传输任务",
      "复制会话",
    ].includes(item);
    if (activeSnapshotStale && sessionBoundItem) return;
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
    if (item === "自定义脚本") {
      setUtilityDialog("custom-scripts");
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
      if (currentActive) requestTerminalSearch();
      else setNotice({ title: "查找", message: "请先打开一个终端会话。" });
      return;
    }
    if (item === "自由输入") {
      if (currentActive) requestTerminalFreeInput();
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
      if (currentActive) requestTerminalGotoLine();
      else setNotice({ title: "跳转到行", message: "请先打开一个终端会话。" });
      return;
    }
    if (item === "导出终端文本" || item === "导出选中文本") {
      void exportTerminalText(item === "导出终端文本" ? "buffer" : "selection");
      return;
    }
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
      openSessionProfileDialog(createSessionDraft(), null, "会话");
      return;
    }
    if (item === "导入会话") {
      setUtilityDialog("session-import");
      return;
    }
    if (item === "新建工作区窗口") {
      void openNewWorkspaceWindow();
      return;
    }
    if (item === "启动会话") {
      if (currentActive) void connectSession(currentActive.profile.id);
      return;
    }
    if (item === "关闭会话") {
      if (currentActive) void disconnectSession(currentActive.profile.id);
      return;
    }
    if (item === "会话设置") {
      if (!currentActive) return;
      openSessionProfileDialog(currentActive.profile, currentActive.profile, "会话");
      return;
    }
    if (["端口转发", "触发器", "密钥管理器"].includes(item)) {
      if (item === "端口转发") {
        if (!currentActive || !isSshLikeProfile(currentActive.profile) || currentActive.runtime.status !== "connected") {
          setNotice({ title: "端口转发", message: "请选择一个已保存并已连接的 SSH/Tmux 会话后再创建 tunnel。" });
          return;
        }
        setUtilityDialog("tunnel");
        return;
      }
      if (item === "触发器") {
        if (!currentActive) return;
        openSessionProfileDialog(currentActive.profile, currentActive.profile, "触发器");
        return;
      }
      setUtilityDialog("keys");
      return;
    }
    if (item === "Sysmon") {
      if (!currentActive) {
        setNotice({ title: "Sysmon", message: "请先选择一个会话。" });
        return;
      }
      setUtilityDialog("sysmon");
      return;
    }
    if (item === "串口分析器") {
      if (!currentActive || currentActive.profile.connection.kind !== "serial") {
        setNotice({ title: "串口分析器", message: "请先选择一个串口会话。" });
        return;
      }
      void openSerialAnalyzer(currentActive);
      return;
    }
    if (item === "Tmux") {
      if (!currentActive || !isSshLikeProfile(currentActive.profile) || currentActive.runtime.status !== "connected") {
        setNotice({ title: "Tmux", message: "请选择一个已连接的 SSH/Tmux 会话后再管理 tmux。" });
        return;
      }
      setUtilityDialog("tmux");
      return;
    }
    if (item === "传输任务") {
      if (!currentActive) {
        setNotice({ title: item, message: "请先选择一个会话。" });
        return;
      }
      setUtilityDialog("transfer");
      return;
    }
    if (item === "复制会话") {
      if (currentActive) duplicateSessionFromContext(currentActive.profile.id);
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
      ? findWorkspacePane(workspaceRootRef.current, paneElement.dataset.paneId)
      : undefined;
    const view = pane ? workspacePaneActiveView(pane) : undefined;
    const nextSessionId = sessionId ?? (activeIdRef.current || sessionsRef.current[0]?.profile.id || null);
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
    const targetId = sessionId ?? contextMenu?.sessionId ?? activeIdRef.current;
    return sessionsRef.current.find((session) => session.profile.id === targetId);
  }

  function currentWorkspaceTarget(target?: {
    paneId?: string | null;
    viewId?: string | null;
    sessionId?: string | null;
  }): CurrentWorkspaceTarget | undefined {
    const panes = workspacePaneLeaves(workspaceRootRef.current);
    const pane = target?.paneId
      ? panes.find((candidate) => candidate.id === target.paneId)
      : target?.viewId
        ? panes.find((candidate) => candidate.views.some((view) => view.id === target.viewId))
        : panes.find((candidate) => candidate.id === activePaneIdRef.current);
    const view = target?.viewId
      ? pane?.views.find((candidate) => candidate.id === target.viewId)
      : pane ? workspacePaneActiveView(pane) : undefined;
    if (!pane || !view || (target?.sessionId && view.sessionId !== target.sessionId)) return undefined;
    const session = sessionsRef.current.find((candidate) => candidate.profile.id === view.sessionId);
    return session ? { pane, view, session } : undefined;
  }

  async function mutateSessionProfileFromContext(
    sessionId: string | null | undefined,
    title: string,
    createProfile: (profile: SessionProfile) => SessionProfile | null,
    activateWorkspace = true,
    successMessage?: (saved: SessionSummary) => string,
  ) {
    const session = contextSession(sessionId);
    if (!session) return;
    const profileId = session.profile.id;
    const token = beginProfileShortcutOperation(profileId);
    if (token === null) return;
    const gate = profileShortcutOperationGateRef.current;
    try {
      const profile = createProfile(session.profile);
      if (!profile) return;
      const saved = await saveProfile(profile, session.profile);
      if (!gate.isCurrent(profileId, token)) return;
      applySavedSession(saved, activateWorkspace);
      if (successMessage) setNotice({ title, message: successMessage(saved) });
    } catch (error) {
      if (gate.isCurrent(profileId, token)) {
        setNotice({ title: `${title}失败`, message: formatError(error) });
      }
    } finally {
      finishProfileShortcutOperation(profileId, token);
    }
  }

  async function renameSessionFromContext(sessionId?: string | null) {
    await mutateSessionProfileFromContext(sessionId, "重命名会话", (profile) => {
      const nextName = window.prompt("标签名称", profile.name);
      return nextName?.trim() ? { ...profile, name: nextName.trim() } : null;
    });
  }

  async function moveSessionToGroupFromContext(sessionId?: string | null) {
    await mutateSessionProfileFromContext(sessionId, "移动会话分组", (profile) => {
      const nextGroup = window.prompt("移动到分组", profile.group || "Sessions");
      return nextGroup === null
        ? null
        : { ...profile, group: nextGroup.trim() || "Sessions" };
    });
  }

  async function saveSessionFromContext(sessionId?: string | null, activateWorkspace = true) {
    await mutateSessionProfileFromContext(
      sessionId,
      "保存会话",
      prepareSessionProfile,
      activateWorkspace,
      (saved) => `已保存 ${saved.profile.name}`,
    );
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
    openSessionProfileDialog(duplicate, null, "会话");
  }

  function openSessionSettingsFromContext(sessionId?: string | null) {
    const session = contextSession(sessionId);
    openSessionProfileDialog(
      session?.profile ?? createSessionDraft(),
      session?.profile ?? null,
      "会话",
    );
  }

  async function writeSessionClipboardText(title: string, text: string) {
    try {
      if (!navigator.clipboard?.writeText) throw new Error("当前环境不支持写入系统剪贴板。");
      await navigator.clipboard.writeText(text);
    } catch (error) {
      setNotice({ title: `${title}失败`, message: formatError(error) });
    }
  }

  function copySessionNameFromContext(sessionId?: string | null) {
    const session = contextSession(sessionId);
    if (!session) return;
    void writeSessionClipboardText("复制会话名称", session.profile.name);
  }

  function copySessionUrlFromContext(sessionId?: string | null) {
    const session = contextSession(sessionId);
    if (!session) return;
    const url = `portmate://sessions/${encodeURIComponent(session.profile.id)}?kind=${encodeURIComponent(session.profile.kind)}&endpoint=${encodeURIComponent(describeProfileEndpoint(session.profile))}`;
    void writeSessionClipboardText("复制会话 URL", url);
  }

  async function exportTerminalText(
    source: TerminalTextExportSource,
    target?: { sessionId: string; viewId: string },
    destination: "default" | "choose" = "default",
  ) {
    const current = currentWorkspaceTarget(target);
    const title = destination === "choose"
      ? "导出终端文本到..."
      : source === "selection" ? "导出选中文本" : "导出终端文本";
    if (!current) {
      if (!target) setNotice({ title, message: "请先打开一个终端视图。" });
      return;
    }
    const { session, view: { id: viewId } } = current;
    const token = beginTerminalExportOperation(viewId);
    if (token === null) return;
    const gate = terminalExportOperationGateRef.current;
    try {
      const destinationPath = destination === "choose" && isBackendAvailable()
        ? await chooseTerminalTextExportPath(
          terminalPrefs.terminalTextExportDirectory,
          terminalTextExportFileName(session.profile.name, source),
        )
        : null;
      if (!gate.isCurrent(viewId, token)) return;
      if (destination === "choose" && isBackendAvailable() && destinationPath === null) return;
      const payload = await requestTerminalTextExport({ sessionId: session.profile.id, viewId, source });
      if (!gate.isCurrent(viewId, token)) return;
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
            destinationDirectory: destination === "default"
              ? normalizeTerminalExportDirectory(terminalPrefs.terminalTextExportDirectory) || null
              : null,
            destinationPath,
            overwrite: destinationPath !== null,
          },
        });
        if (!gate.isCurrent(viewId, token)) return;
        setNotice({
          title,
          message: `${formatBytes(result.size)} · ${payload.lineCount} 行 · SHA-256 ${result.sha256.slice(0, 16)}...\n${result.path}`,
        });
      } else {
        const { downloadTerminalText } = await import("./terminal-export-download");
        if (!gate.isCurrent(viewId, token)) return;
        const fileName = downloadTerminalText(payload.text, session.profile.name, source);
        setNotice({ title, message: `已下载 ${fileName} · ${formatBytes(payload.bytes)} · ${payload.lineCount} 行` });
      }
    } catch (error) {
      if (gate.isCurrent(viewId, token)) setNotice({ title, message: formatError(error) });
    } finally {
      finishTerminalExportOperation(viewId, token);
    }
  }

  async function runTerminalSelectionAction(
    action: TerminalSelectionAction,
    title: string,
    target?: { sessionId: string; viewId: string },
  ) {
    const current = currentWorkspaceTarget(target);
    if (!current) {
      if (!target) setNotice({ title, message: "请先打开一个终端视图。" });
      return;
    }
    const commandTarget = {
      paneId: current.pane.id,
      sessionId: current.session.profile.id,
      viewId: current.view.id,
    };
    try {
      const { executeTerminalSelectionAction } = await import("./terminal-selection-event");
      if (!currentWorkspaceTarget(commandTarget)) return;
      await executeTerminalSelectionAction({ sessionId: commandTarget.sessionId, viewId: commandTarget.viewId, action });
    } catch (error) {
      if (currentWorkspaceTarget(commandTarget)) setNotice({ title, message: formatError(error) });
    }
  }

  async function searchTerminalOnline(target?: { sessionId: string; viewId: string }) {
    const current = currentWorkspaceTarget(target);
    if (!current) {
      if (!target) setNotice({ title: "在线搜索", message: "请先打开一个终端视图。" });
      return;
    }
    const commandTarget = {
      paneId: current.pane.id,
      sessionId: current.session.profile.id,
      viewId: current.view.id,
    };
    try {
      const { executeTerminalOnlineSearch } = await import("./terminal-selection-event");
      if (!currentWorkspaceTarget(commandTarget)) return;
      await executeTerminalOnlineSearch({
        sessionId: commandTarget.sessionId,
        viewId: commandTarget.viewId,
        fallback: current.session.lastLine,
      });
    } catch (error) {
      if (currentWorkspaceTarget(commandTarget)) setNotice({ title: "在线搜索", message: formatError(error) });
    }
  }

  async function runTerminalBufferAction(
    action: TerminalBufferAction,
    title: string,
    target?: { sessionId: string; viewId: string },
  ) {
    const current = currentWorkspaceTarget(target);
    if (!current) {
      if (!target) setNotice({ title, message: "请先打开一个终端视图。" });
      return;
    }
    const commandTarget = {
      paneId: current.pane.id,
      sessionId: current.session.profile.id,
      viewId: current.view.id,
    };
    try {
      const { executeTerminalBufferAction } = await import("./terminal-buffer-event");
      if (!currentWorkspaceTarget(commandTarget)) return;
      await executeTerminalBufferAction({ sessionId: commandTarget.sessionId, viewId: commandTarget.viewId, action });
    } catch (error) {
      if (currentWorkspaceTarget(commandTarget)) setNotice({ title, message: formatError(error) });
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

  function invalidateDeletedSessionProfileOperations(profileId: string) {
    invalidateTerminalInputSession(profileId);
    sessionSummaryRefreshGateRef.current.invalidate("summaries");
    connectionAttemptGateRef.current.invalidate(profileId);
    connectionCloseGateRef.current.invalidate(profileId);
    keyManagerProfileMutationGateRef.current.invalidate(profileId);
    sessionSettingsProfileMutationGateRef.current.invalidate(profileId);
    invalidateProfileShortcutOperation(profileId);
    invalidateTerminalExportsForSession(profileId);
    const detachedViewIds = workspacePaneLeaves(workspaceRootRef.current)
      .flatMap((pane) => pane.views)
      .filter((view) => view.sessionId === profileId)
      .map((view) => view.id);
    invalidateWorkspaceViewDetachOperations(detachedViewIds);
    setDisconnectingSessionIds((current) => {
      if (!current.has(profileId)) return current;
      const next = new Set(current);
      next.delete(profileId);
      return next;
    });
    const credentialRequest = credentialResolverRef.current;
    if (credentialRequest?.sessionId === profileId) {
      credentialResolverRef.current = null;
      setCredentialPrompt(null);
      credentialRequest.resolve(null);
    }
    if (hostKeyPromptRef.current?.profile.id === profileId) {
      hostKeyPromptOperationGateRef.current.invalidateAll();
      setHostKeyPrompt(null);
    }
    serialCaptureOperationGateRef.current.invalidate(profileId);
    serialCaptureActionTokensRef.current.delete(profileId);
    setSerialCaptureActionIds((current) => {
      if (!current.has(profileId)) return current;
      const next = new Set(current);
      next.delete(profileId);
      return next;
    });
    serialControlOperationGateRef.current.invalidate(profileId);
    serialAnalyzerWindowOperationGateRef.current.invalidate(profileId);
    setSerialControlBusyIds((current) => {
      if (!current.has(profileId)) return current;
      const next = new Set(current);
      next.delete(profileId);
      return next;
    });
    activeLogRefreshGateRef.current.invalidate(profileId);
    delete logSignatureRef.current[profileId];
    sessionsSignatureRef.current = "";
  }

  function invalidateWorkspaceViewDetachOperations(viewIds?: readonly string[]) {
    const gate = detachedWindowOperationGateRef.current;
    if (viewIds === undefined) {
      gate.invalidateAll();
      setDetachingWorkspaceViewIds((current) => current.size ? new Set() : current);
      return;
    }
    if (!viewIds.length) return;
    const invalidated = new Set(viewIds);
    for (const viewId of invalidated) gate.invalidate(viewId);
    setDetachingWorkspaceViewIds((current) => {
      const next = new Set([...current].filter((viewId) => !invalidated.has(viewId)));
      return next.size === current.size ? current : next;
    });
  }

  function applyDeletedSessionProfile(response: DeleteSessionProfileResponse) {
    const profileId = response.deletedProfileId;
    const remainingSessionIds = response.sessions.map((session) => session.profile.id);
    const reconciled = reconcileWorkspaceSnapshot({
      version: 4,
      root: workspaceRootRef.current,
      activePaneId: activePaneIdRef.current,
      activeId: activeIdRef.current,
      tabColors: tabColorsRef.current,
    }, remainingSessionIds, { fallbackToFirst: !workspaceWindowId });

    invalidateDeletedSessionProfileOperations(profileId);
    setSessions(response.sessions);
    saveLocalSessionSummaries(response.sessions);
    updateOneKeys(response.oneKeys);
    updateHostKeys(response.hostKeys);
    updateGrants(response.grants);
    setLogs((current) => Object.fromEntries(
      Object.entries(current).filter(([sessionId]) => sessionId !== profileId),
    ));
    updateTransfers((current) => current.filter((transfer) => transfer.sessionId !== profileId));
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
    if (draftExpectedProfileRef.current?.id === profileId) {
      draftExpectedProfileRef.current = null;
    }
    setDraft((current) => current.id === profileId ? createSessionDraft() : current);
    delete serialCapturesRef.current[profileId];
  }

  async function deleteSessionFromContext(sessionId?: string | null) {
    const target = contextSession(sessionId);
    if (!target) return;
    const profileId = target.profile.id;
    const token = beginProfileShortcutOperation(profileId);
    if (token === null) return;
    const confirmed = window.confirm(
      `删除会话 Profile “${target.profile.name}”？\n\n活动连接会先断开；内存历史、传输记录、Profile 级 Host Key 和会话绑定会删除。磁盘日志分片与安全审计保留。`,
    );
    if (!confirmed) {
      finishProfileShortcutOperation(profileId, token);
      return;
    }

    pendingProfileDeletionRef.current.set(profileId, { token, profileName: target.profile.name });
    const gate = profileShortcutOperationGateRef.current;
    try {
      const response = isBackendAvailable()
        ? await invokeBackend<DeleteSessionProfileResponse>("delete_session_profile", { sessionId: profileId })
        : deleteSessionProfileFromClientState(profileId, { sessions: sessionsRef.current, oneKeys, hostKeys, grants });
      if (!gate.isCurrent(profileId, token)) return;
      pendingProfileDeletionRef.current.delete(profileId);
      applyDeletedSessionProfile(response);
      setNotice({ title: "会话已删除", message: `已删除 ${target.profile.name}；磁盘日志仍可在日志管理器中查看或清理。` });
    } catch (error) {
      if (gate.isCurrent(profileId, token)) {
        pendingProfileDeletionRef.current.delete(profileId);
        setNotice({ title: "删除会话失败", message: formatError(error) });
      }
    } finally {
      if (pendingProfileDeletionRef.current.get(profileId)?.token === token) {
        pendingProfileDeletionRef.current.delete(profileId);
      }
      finishProfileShortcutOperation(profileId, token);
    }
  }

  function closeSideSessionsFromContext(sessionId?: string | null) {
    const target = contextSession(sessionId);
    if (!target) return;
    const currentSessions = sessionsRef.current;
    const index = currentSessions.findIndex((session) => session.profile.id === target.profile.id);
    if (index < 0) return;
    const rightIds = currentSessions.slice(index + 1).map((session) => session.profile.id);
    void closeSessionsByIds(rightIds);
  }

  function handleContextMenuAction(action: SessionContextAction, sessionId?: string | null) {
    setContextMenu(null);
    const target = contextSession(sessionId);
    if (sessionId && !target) return;
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
        if (target) void reconnectSession(target.profile.id);
        return;
      case "save":
        void saveSessionFromContext(sessionId);
        return;
      case "split-h":
        if (target) {
          splitWorkspace(
            workspaceSplitDirectionForVisualOrientation("horizontal"),
            "second",
            findWorkspacePaneBySession(workspaceRootRef.current, target.profile.id)?.id,
            target.profile.id,
          );
        }
        return;
      case "split-v":
        if (target) {
          splitWorkspace(
            workspaceSplitDirectionForVisualOrientation("vertical"),
            "second",
            findWorkspacePaneBySession(workspaceRootRef.current, target.profile.id)?.id,
            target.profile.id,
          );
        }
        return;
      case "move-group":
        void moveSessionToGroupFromContext(sessionId);
        return;
      case "close":
        if (target) void disconnectSession(target.profile.id);
        return;
      case "close-all":
        void closeSessionsByIds(sessionsRef.current.map((session) => session.profile.id));
        return;
      case "close-inactive":
        void closeSessionsByIds(sessionsRef.current
          .filter((session) => session.profile.id !== activeIdRef.current)
          .map((session) => session.profile.id));
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
    const current = currentWorkspaceTarget(state);
    if (!current) return;
    activateWorkspacePane(current.pane.id, current.view.id);
    const target = { sessionId: current.session.profile.id, viewId: current.view.id };
    switch (action) {
      case "copy":
        void runTerminalSelectionAction("copy", "复制", target);
        return;
      case "paste":
        void navigator.clipboard?.readText().then((text) => {
          if (text && currentWorkspaceTarget({ ...target, paneId: current.pane.id })) {
            return routeTerminalInput(target.sessionId, text, "atomic");
          }
        }).catch((error) => {
          if (currentWorkspaceTarget({ ...target, paneId: current.pane.id })) {
            setNotice({ title: "粘贴", message: formatError(error) });
          }
        });
        return;
      case "find":
        window.requestAnimationFrame(() => {
          if (currentWorkspaceTarget({ ...target, paneId: current.pane.id })) requestTerminalSearch();
        });
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
      case "export-buffer-to":
        void exportTerminalText("buffer", target, "choose");
        return;
      case "export-selection":
        void exportTerminalText("selection", target);
        return;
      case "triggers": {
        openSessionProfileDialog(
          current.session.profile,
          current.session.profile,
          "触发器",
        );
        return;
      }
    }
  }

  function openNewSessionDialog() {
    openSessionProfileDialog(createSessionDraft(), null, "会话");
  }

  function openSessionProfileDialog(
    nextDraft: SessionProfile,
    expectedProfile: SessionProfile | null,
    section: string,
  ) {
    setDraft(cloneSessionProfile(nextDraft));
    draftExpectedProfileRef.current = expectedProfile
      ? cloneSessionProfile(expectedProfile)
      : null;
    setSessionSettingsMode(expectedProfile ? "edit" : "create");
    setSessionSettingsSection(section);
    setDialog("session");
  }

  function activateSession(sessionId: string) {
    const currentRoot = workspaceRootRef.current;
    const currentActivePaneId = activePaneIdRef.current;
    const currentPane = findWorkspacePane(currentRoot, currentActivePaneId);
    const existingPane = currentPane?.sessionIds.includes(sessionId)
      ? currentPane
      : findWorkspacePaneBySession(currentRoot, sessionId);
    if (existingPane) {
      setWorkspaceRoot((current) => activateWorkspacePaneSession(current, existingPane.id, sessionId));
      setActivePaneId(existingPane.id);
      setActiveId(sessionId);
      setZoomedPaneId((current) => current ? existingPane.id : "");
      return;
    }
    if (!currentRoot) {
      const pane = createWorkspacePane(sessionId);
      setWorkspaceRoot(pane);
      setActivePaneId(pane.id);
    } else {
      const targetPane = currentPane ?? workspacePaneLeaves(currentRoot)[0];
      if (targetPane) {
        const nextRoot = addWorkspacePaneSession(currentRoot, targetPane.id, sessionId);
        if (nextRoot === currentRoot && !targetPane.sessionIds.includes(sessionId)) {
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
    const pane = findWorkspacePane(workspaceRootRef.current, paneId);
    const view = pane?.views.find((candidate) => candidate.id === viewId);
    if (!pane || !view) return;
    setWorkspaceRoot((current) => activateWorkspacePaneView(current, paneId, viewId));
    setActivePaneId(paneId);
    setActiveId(view.sessionId);
    setZoomedPaneId((current) => current ? paneId : "");
  }

  function cycleActiveWorkspaceView(offset: -1 | 1) {
    const pane = findWorkspacePane(workspaceRootRef.current, activePaneIdRef.current);
    const view = pane ? workspacePaneViewAtOffset(pane, offset) : undefined;
    if (!pane || !view) return;
    if (view.id !== pane.activeViewId) activateWorkspacePane(pane.id, view.id);
    focusWorkspacePaneInput(pane.id);
  }

  function setActiveWorkspaceViewKeyMode(
    keyMode: TerminalKeyMode,
    paneId?: string,
    viewId?: string,
  ) {
    const targetPaneId = paneId ?? activePaneIdRef.current;
    const targetViewId = viewId ?? findWorkspacePane(workspaceRootRef.current, targetPaneId)?.activeViewId;
    if (!targetPaneId || !targetViewId) return;
    setWorkspaceRoot((current) => setWorkspacePaneViewKeyMode(current, targetPaneId, targetViewId, keyMode));
    focusWorkspacePaneInput(targetPaneId);
  }

  function restoreWorkspaceLayout() {
    const restored = reconcileWorkspaceSnapshot(
      loadWorkspaceSnapshot(workspaceStorageKey),
      sessionsRef.current.map((session) => session.profile.id),
      { fallbackToFirst: !workspaceWindowId },
    );
    invalidateWorkspaceViewDetachOperations();
    invalidateAllTerminalExportOperations();
    setWorkspaceRoot(restored.root);
    setActivePaneId(restored.activePaneId);
    setActiveId(restored.activeId);
    setTabColors(restored.tabColors);
    setZoomedPaneId("");
  }

  function splitWorkspace(
    direction: WorkspaceSplitDirection,
    placement: WorkspaceSplitPlacement = "second",
    paneId?: string,
    sessionId?: string,
  ) {
    const currentSessions = sessionsRef.current;
    if (sessionId && !currentSessions.some((session) => session.profile.id === sessionId)) return;
    const currentActiveId = activeIdRef.current;
    const primaryId = sessionId
      ?? (currentSessions.some((session) => session.profile.id === currentActiveId) ? currentActiveId : currentSessions[0]?.profile.id);
    if (!primaryId) {
      openNewSessionDialog();
      return;
    }
    const currentRoot = workspaceRootRef.current;
    const targetPaneId = paneId ?? activePaneIdRef.current;
    const root = currentRoot ?? createWorkspacePane(primaryId, targetPaneId || createWorkspaceNodeId("pane"));
    const panes = workspacePaneLeaves(root);
    if (panes.length >= MAX_WORKSPACE_PANES) {
      setNotice({ title: "分屏", message: `最多同时打开 ${MAX_WORKSPACE_PANES} 个窗格。` });
      return;
    }
    const targetPane = findWorkspacePane(root, targetPaneId)
      ?? findWorkspacePaneBySession(root, primaryId)
      ?? panes[0];
    if (!targetPane) return;
    const openSessionIds = new Set(panes.flatMap((pane) => pane.views.map((view) => view.sessionId)));
    const nextId = currentSessions.find((session) => !openSessionIds.has(session.profile.id))?.profile.id ?? primaryId;
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

  function duplicateActiveWorkspaceView(paneId?: string, viewId?: string) {
    const current = currentWorkspaceTarget({ paneId: paneId ?? activePaneIdRef.current, viewId });
    if (!current) return;
    const { pane, view: source } = current;
    if (pane.views.length >= MAX_WORKSPACE_GROUP_TABS) {
      setNotice({ title: "复制视图", message: `每个分组最多包含 ${MAX_WORKSPACE_GROUP_TABS} 个视图。` });
      return;
    }
    const currentSessions = sessionsRef.current;
    const sessionName = current.session.profile.name;
    const baseTitle = source.title || sessionName;
    const labels = new Set(pane.views.map((view) => (
      view.title || currentSessions.find((session) => session.profile.id === view.sessionId)?.profile.name || "会话"
    )));
    let duplicateTitle = `${baseTitle} 副本`;
    let suffix = 2;
    while (labels.has(duplicateTitle)) {
      duplicateTitle = `${baseTitle} 副本 ${suffix}`;
      suffix += 1;
    }
    const duplicateId = createWorkspaceNodeId("view");
    const currentRoot = workspaceRootRef.current;
    const nextRoot = duplicateWorkspacePaneView(currentRoot, pane.id, source.id, duplicateId, duplicateTitle);
    if (nextRoot === currentRoot) return;
    setWorkspaceRoot(nextRoot);
    setActivePaneId(pane.id);
    setActiveId(source.sessionId);
    focusWorkspacePaneInput(pane.id);
  }

  function openWorkspaceViewRename(paneId?: string, viewId?: string) {
    const current = currentWorkspaceTarget({ paneId: paneId ?? activePaneIdRef.current, viewId });
    if (!current) return;
    const { pane, view, session } = current;
    const sessionName = session.profile.name;
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
    const current = currentWorkspaceTarget({ paneId, viewId });
    if (!current) return;
    const { pane, view } = current;
    activateWorkspacePane(pane.id, view.id);
    setOpenMenu(null);
    setContextMenu(null);
    setWorkspaceViewContextMenu({ x, y, paneId: pane.id, viewId: view.id });
  }

  function changeWorkspaceViewColor(paneId: string, viewId: string, color: string) {
    if (!currentWorkspaceTarget({ paneId, viewId })) return;
    setWorkspaceRoot((current) => setWorkspacePaneViewColor(current, paneId, viewId, color));
    setWorkspaceViewContextMenu(null);
    focusWorkspacePaneInput(paneId);
  }

  function closeWorkspaceViews(paneId: string, viewIds: string[]) {
    const currentRoot = workspaceRootRef.current;
    const panes = workspacePaneLeaves(currentRoot);
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

    let nextRoot = currentRoot;
    for (const item of closedViews) {
      nextRoot = removeWorkspacePaneView(nextRoot, paneId, item.view.id);
    }
    if (nextRoot === currentRoot) return;
    const nextPanes = workspacePaneLeaves(nextRoot);
    const sourceIndex = panes.findIndex((pane) => pane.id === paneId);
    const currentActivePaneId = activePaneIdRef.current;
    const nextActive = nextPanes.find((pane) => pane.id === currentActivePaneId)
      ?? nextPanes[Math.min(Math.max(0, sourceIndex), nextPanes.length - 1)];
    if (!nextActive) return;
    for (const item of closedViews) invalidateTerminalExportOperation(item.view.id);
    invalidateWorkspaceViewDetachOperations(closedViews.map((item) => item.view.id));
    const shouldRefocus = nextActive.id !== currentActivePaneId
      || (paneId === currentActivePaneId && closedViews.some((item) => item.view.id === source.activeViewId));
    setWorkspaceRoot(nextRoot);
    setActivePaneId(nextActive.id);
    setActiveId(workspacePaneActiveView(nextActive).sessionId);
    setZoomedPaneId((current) => current && !findWorkspacePane(nextRoot, current) ? nextActive.id : current);
    pushClosedWorkspaceViews(closedViews);
    if (shouldRefocus) focusWorkspacePaneInput(nextActive.id);
  }

  function closeActiveWorkspaceView() {
    const pane = findWorkspacePane(workspaceRootRef.current, activePaneIdRef.current);
    if (pane) closeWorkspaceViews(pane.id, [pane.activeViewId]);
  }

  function closeOtherWorkspaceViews(paneId = activePaneIdRef.current, viewId?: string) {
    const pane = findWorkspacePane(workspaceRootRef.current, paneId);
    if (!pane) return;
    const activeViewId = viewId ?? pane.activeViewId;
    const viewIds = pane.views.filter((view) => view.id !== activeViewId).map((view) => view.id);
    if (!viewIds.length) {
      setNotice({ title: "关闭其他视图", message: "当前分组没有其他视图。" });
      return;
    }
    closeWorkspaceViews(pane.id, viewIds);
  }

  function closeRightWorkspaceViews(paneId = activePaneIdRef.current, viewId?: string) {
    const pane = findWorkspacePane(workspaceRootRef.current, paneId);
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
    const history = closedWorkspaceViewsRef.current;
    const currentSessions = sessionsRef.current;
    let historyIndex = history.length - 1;
    while (historyIndex >= 0 && !currentSessions.some((session) => session.profile.id === history[historyIndex].view.sessionId)) {
      historyIndex -= 1;
    }
    if (historyIndex < 0) {
      setClosedWorkspaceViews([]);
      setNotice({ title: "重新打开已关闭视图", message: "没有可重新打开的视图。" });
      return;
    }
    const closedView = history[historyIndex];
    const currentRoot = workspaceRootRef.current;
    const panes = workspacePaneLeaves(currentRoot);
    const candidateIds = [closedView.paneId, activePaneIdRef.current, ...panes.map((pane) => pane.id)];
    const target = candidateIds
      .filter((paneId, index) => paneId && candidateIds.indexOf(paneId) === index)
      .map((paneId) => panes.find((pane) => pane.id === paneId))
      .find((pane) => pane && (
        pane.views.length < MAX_WORKSPACE_GROUP_TABS
      ));
    if (!target || !currentRoot) {
      setNotice({ title: "重新打开已关闭视图", message: `所有分组均已达到 ${MAX_WORKSPACE_GROUP_TABS} 个视图。` });
      return;
    }
    const insertionIndex = target.id === closedView.paneId
      ? closedView.index
      : target.views.length;
    const nextRoot = insertWorkspacePaneView(
      currentRoot,
      target.id,
      closedView.view,
      insertionIndex,
    );
    if (nextRoot === currentRoot) return;
    setWorkspaceRoot(nextRoot);
    setActivePaneId(target.id);
    setActiveId(closedView.view.sessionId);
    setZoomedPaneId((current) => current ? target.id : "");
    setClosedWorkspaceViews(history.slice(0, historyIndex));
    focusWorkspacePaneInput(target.id);
  }

  function handleWorkspaceViewContextAction(
    action: WorkspaceViewContextAction,
    paneId: string,
    viewId: string,
  ) {
    setWorkspaceViewContextMenu(null);
    const current = currentWorkspaceTarget({ paneId, viewId });
    if (!current) return;
    const { pane, view } = current;
    switch (action) {
      case "copy-name":
        copySessionNameFromContext(view.sessionId);
        return;
      case "copy-url":
        copySessionUrlFromContext(view.sessionId);
        return;
      case "reconnect":
        void reconnectSession(view.sessionId, false);
        return;
      case "save":
        void saveSessionFromContext(view.sessionId, false);
        return;
      case "export-buffer":
        void exportTerminalText("buffer", { sessionId: view.sessionId, viewId: view.id });
        return;
      case "export-buffer-to":
        void exportTerminalText("buffer", { sessionId: view.sessionId, viewId: view.id }, "choose");
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

  function closeWorkspacePane(paneId?: string, recordHistory = true) {
    const currentRoot = workspaceRootRef.current;
    const currentActivePaneId = activePaneIdRef.current;
    const targetPaneId = paneId ?? currentActivePaneId;
    const panes = workspacePaneLeaves(currentRoot);
    if (panes.length <= 1 || !targetPaneId) return;
    const removedIndex = panes.findIndex((pane) => pane.id === targetPaneId);
    if (removedIndex < 0) return;
    const removedPane = panes[removedIndex];
    invalidateWorkspaceViewDetachOperations(removedPane.views.map((view) => view.id));
    const nextRoot = removeWorkspacePane(currentRoot, targetPaneId);
    const nextPanes = workspacePaneLeaves(nextRoot);
    const currentActive = nextPanes.find((pane) => pane.id === currentActivePaneId);
    const nextActive = currentActive ?? nextPanes[Math.min(removedIndex, nextPanes.length - 1)];
    setWorkspaceRoot(nextRoot);
    setActivePaneId(nextActive?.id ?? "");
    setActiveId(nextActive ? workspacePaneActiveView(nextActive).sessionId : activeIdRef.current);
    setZoomedPaneId((current) => current ? nextActive?.id ?? "" : "");
    if (recordHistory) {
      pushClosedWorkspaceViews(removedPane.views.map((view, index) => ({ view, paneId: targetPaneId, index })));
    }
  }

  function splitWorkspaceViewToGroup(
    direction: WorkspaceSplitDirection,
    placement: WorkspaceSplitPlacement,
    sourcePaneId = activePaneIdRef.current,
    sourceViewId?: string,
  ) {
    const currentRoot = workspaceRootRef.current;
    const panes = workspacePaneLeaves(currentRoot);
    const source = findWorkspacePane(currentRoot, sourcePaneId);
    if (!currentRoot || !source) return;
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
      currentRoot,
      source.id,
      activeView.id,
      direction,
      newPaneId,
      createWorkspaceNodeId("split"),
      placement,
    );
    if (nextRoot === currentRoot) {
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
    const currentRoot = workspaceRootRef.current;
    const source = findWorkspacePane(currentRoot, sourcePaneId);
    const target = findWorkspacePane(currentRoot, targetPaneId);
    const view = source?.views.find((candidate) => candidate.id === viewId);
    if (!currentRoot || !source || !target || !view) return;
    if (sourcePaneId === targetPaneId && source.views.length <= 1) {
      setNotice({ title: "拖放视图", message: "最终视图不能拆成空分组。" });
      return;
    }
    const paneDelta = source.views.length > 1 ? 1 : 0;
    if (workspacePaneLeaves(currentRoot).length + paneDelta > MAX_WORKSPACE_PANES) {
      setNotice({ title: "拖放视图", message: `工作区最多支持 ${MAX_WORKSPACE_PANES} 个分组。` });
      return;
    }
    const direction: WorkspaceSplitDirection = edge === "left" || edge === "right" ? "vertical" : "horizontal";
    const placement: WorkspaceSplitPlacement = edge === "left" || edge === "up" ? "first" : "second";
    const newPaneId = createWorkspaceNodeId("pane");
    const nextRoot = moveWorkspacePaneViewToNewGroup(
      currentRoot,
      sourcePaneId,
      targetPaneId,
      viewId,
      direction,
      newPaneId,
      createWorkspaceNodeId("split"),
      placement,
    );
    if (nextRoot === currentRoot) {
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
    const source = findWorkspacePane(workspaceRootRef.current, sourcePaneId);
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
    const currentRoot = workspaceRootRef.current;
    const source = findWorkspacePane(currentRoot, sourcePaneId);
    const target = findWorkspacePane(currentRoot, targetPaneId);
    const view = source?.views.find((candidate) => candidate.id === viewId);
    if (!source || !target || !view) return;
    if (sourcePaneId !== targetPaneId && target.views.length >= MAX_WORKSPACE_GROUP_TABS) {
      setNotice({ title: "移动视图到分组", message: `每个分组最多包含 ${MAX_WORKSPACE_GROUP_TABS} 个视图。` });
      return;
    }
    const nextRoot = moveWorkspacePaneView(currentRoot, sourcePaneId, targetPaneId, view.id, targetIndex);
    if (nextRoot === currentRoot) return;
    setWorkspaceRoot(nextRoot);
    setActivePaneId(targetPaneId);
    setActiveId(view.sessionId);
    setZoomedPaneId((current) => current ? targetPaneId : "");
    setWorkspaceGroupMove(null);
    focusWorkspacePaneInput(targetPaneId);
  }

  function mergeWorkspaceGroup(sourcePaneId: string, targetPaneId: string) {
    const currentRoot = workspaceRootRef.current;
    const source = findWorkspacePane(currentRoot, sourcePaneId);
    const target = findWorkspacePane(currentRoot, targetPaneId);
    if (!source || !target || sourcePaneId === targetPaneId) return;
    if (target.views.length + source.views.length > MAX_WORKSPACE_GROUP_TABS) {
      setNotice({ title: "合并当前分组", message: `每个分组最多包含 ${MAX_WORKSPACE_GROUP_TABS} 个视图。` });
      return;
    }
    const nextRoot = mergeWorkspacePaneGroups(currentRoot, sourcePaneId, targetPaneId);
    if (nextRoot === currentRoot) return;
    setWorkspaceRoot(nextRoot);
    setActivePaneId(targetPaneId);
    setActiveId(workspacePaneActiveView(source).sessionId);
    setZoomedPaneId("");
    setWorkspaceGroupMove(null);
    focusWorkspacePaneInput(targetPaneId);
  }

  async function openNewWorkspaceWindow() {
    const windowId = createWorkspaceNodeId("pane")
      .replace(/^pane-/, "workspace-")
      .replace(/[^A-Za-z0-9_-]/g, "-");
    try {
      await openWorkspaceWindow(windowId);
    } catch (error) {
      setNotice({ title: "新建工作区窗口失败", message: formatError(error) });
    }
  }

  async function detachWorkspacePane(paneId?: string) {
    const panes = workspacePaneLeaves(workspaceRootRef.current);
    const pane = panes.find((item) => item.id === (paneId ?? activePaneIdRef.current));
    const activeView = pane ? workspacePaneActiveView(pane) : undefined;
    const session = activeView ? sessionsRef.current.find((item) => item.profile.id === activeView.sessionId) : undefined;
    if (!pane || !activeView || !session) return;
    if (panes.length <= 1 && pane.views.length <= 1) {
      setNotice({ title: "移到新窗口", message: "主窗口中至少需要保留一个窗格或视图。" });
      return;
    }
    const gate = detachedWindowOperationGateRef.current;
    const token = gate.begin(activeView.id);
    if (token === null) return;
    setDetachingWorkspaceViewIds((current) => new Set(current).add(activeView.id));
    const request: DetachedPaneRequest = {
      windowId: createWorkspaceNodeId("pane").replace(/[^A-Za-z0-9_-]/g, "-"),
      ownerWindowId,
      paneId: pane.id,
      viewId: activeView.id,
      sessionId: activeView.sessionId,
      title: activeView.title,
      color: activeView.color,
      keyMode: activeView.keyMode,
    };
    try {
      const controller = await openDetachedPaneWindow(request, activeView.title || session.profile.name);
      if (!gate.isCurrent(activeView.id, token)) {
        await controller.close();
        return;
      }
      const committed = commitWorkspaceViewDetach(
        workspaceRootRef.current,
        activePaneIdRef.current,
        activeView.id,
        activeView.sessionId,
      );
      if (committed.status === "last-view") {
        await controller.close();
        setNotice({
          title: "移到新窗口",
          message: "窗口创建期间工作区布局已变化；主窗口必须保留一个视图，请重试。",
        });
        return;
      }
      if (committed.status === "detached") {
        workspaceRootRef.current = committed.root;
        activePaneIdRef.current = committed.activePaneId;
        setWorkspaceRoot(committed.root);
        setActivePaneId(committed.activePaneId);
        setActiveId(committed.activeId);
        setZoomedPaneId((current) => current ? committed.activePaneId : "");
        focusWorkspacePaneInput(committed.activePaneId);
      }
    } catch (error) {
      if (gate.isCurrent(activeView.id, token)) {
        setNotice({ title: "移到新窗口失败", message: formatError(error) });
      }
    } finally {
      if (gate.finish(activeView.id, token)) {
        setDetachingWorkspaceViewIds((current) => {
          if (!current.has(activeView.id)) return current;
          const next = new Set(current);
          next.delete(activeView.id);
          return next;
        });
      }
    }
  }

  async function openSerialAnalyzer(session: SessionSummary) {
    const currentSession = sessionsRef.current.find((candidate) => candidate.profile.id === session.profile.id);
    if (!currentSession || currentSession.profile.connection.kind !== "serial") return;
    const sessionId = currentSession.profile.id;
    const gate = serialAnalyzerWindowOperationGateRef.current;
    const token = gate.begin(sessionId);
    if (token === null) return;
    const isCurrent = () => gate.isCurrent(sessionId, token)
      && sessionsRef.current.some((candidate) => (
        candidate.profile.id === sessionId && candidate.profile.connection.kind === "serial"
      ));
    const request: SerialAnalyzerRequest = {
      windowId: createWorkspaceNodeId("pane").replace(/^pane-/, "serial-analyzer-").replace(/[^A-Za-z0-9_-]/g, "-"),
      ownerWindowId,
      sessionId,
    };
    try {
      const { openSerialAnalyzerWindow } = await import("./serial-analyzer-window");
      const latestSession = sessionsRef.current.find((candidate) => candidate.profile.id === request.sessionId);
      if (!isCurrent() || !latestSession || latestSession.profile.connection.kind !== "serial") return;
      const controller = await openSerialAnalyzerWindow(request, latestSession.profile.name, isCurrent);
      if (!isCurrent()) {
        try {
          await controller.close();
        } catch (error) {
          setNotice({ title: "关闭失效的串口分析器失败", message: formatError(error) });
        }
      }
    } catch (error) {
      if (isCurrent()) setNotice({ title: "打开串口分析器失败", message: formatError(error) });
    } finally {
      gate.finish(sessionId, token);
    }
  }

  function reattachDetachedPane(command: DetachedPaneCommand): DetachedPaneResult {
    const session = sessionsRef.current.find((item) => item.profile.id === command.sessionId);
    if (!session) {
      const error = "原会话已不存在。";
      setNotice({ title: "返回主窗口失败", message: error });
      return { windowId: command.windowId, requestId: command.requestId, action: "reattach", ok: false, error };
    }
    const returnedView: WorkspaceView = { id: command.viewId, sessionId: command.sessionId, title: command.title, color: command.color, keyMode: command.keyMode };
    const committed = commitWorkspaceViewReattach(
      workspaceRootRef.current,
      activePaneIdRef.current,
      command.paneId,
      returnedView,
    );
    if (committed.status === "conflict") {
      const error = "返回的视图标识与当前工作区冲突。";
      setNotice({ title: "返回主窗口失败", message: error });
      return { windowId: command.windowId, requestId: command.requestId, action: "reattach", ok: false, error };
    }
    setWorkspaceRoot(committed.root);
    setActivePaneId(committed.activePaneId);
    setActiveId(committed.activeId);
    if (committed.replaced) pushClosedWorkspaceViews([committed.replaced]);
    if (committed.placement !== "existing-view" && committed.placement !== "original-pane" && committed.placement !== "empty-workspace") {
      setZoomedPaneId("");
    }
    focusWorkspacePaneInput(committed.activePaneId);
    if (committed.placement === "original-pane" && committed.replaced) {
      setNotice({ title: "窗格已返回", message: `原分组已达到 ${MAX_WORKSPACE_GROUP_TABS} 个视图，已替换该分组的活动视图。` });
    } else if (committed.placement === "max-panes") {
      setNotice({ title: "窗格已返回", message: `工作区已达到 ${MAX_WORKSPACE_PANES} 个窗格，已在当前窗格打开返回的会话。` });
    } else if (committed.placement === "max-depth") {
      setNotice({ title: "窗格已返回", message: `所有窗格均已达到 ${MAX_WORKSPACE_DEPTH} 层深度，已在当前窗格打开返回的会话。` });
    }
    return { windowId: command.windowId, requestId: command.requestId, action: "reattach", ok: true, error: "" };
  }

  function swapWorkspacePane(direction: WorkspacePaneDirection, sourcePaneId = activePaneIdRef.current) {
    const currentRoot = workspaceRootRef.current;
    const nextPane = findWorkspacePaneInDirection(currentRoot, sourcePaneId, direction);
    if (!nextPane) {
      setNotice({ title: "交换窗格", message: "该方向没有可交换的窗格。" });
      return;
    }
    setWorkspaceRoot(swapWorkspacePanes(currentRoot, sourcePaneId, nextPane.id));
    focusWorkspacePaneInput(sourcePaneId);
  }

  function toggleWorkspaceZoom(paneId = activePaneIdRef.current) {
    const currentRoot = workspaceRootRef.current;
    if (workspacePaneLeaves(currentRoot).length <= 1 || !findWorkspacePane(currentRoot, paneId)) return;
    setZoomedPaneId((current) => current === paneId ? "" : paneId);
    focusWorkspacePaneInput(paneId);
  }

  async function saveDraft(proxyPasswordUpdate: ProxyPasswordUpdate = null): Promise<SessionSummary | null> {
    const profile = prepareSessionProfile(draft);
    const gate = sessionSettingsProfileMutationGateRef.current;
    const token = gate.begin(profile.id);
    if (token === null) return null;
    try {
      const saved = await saveProfile(
        profile,
        draftExpectedProfileRef.current,
        proxyPasswordUpdate,
      );
      if (!gate.isCurrent(profile.id, token)) return null;
      applySavedSession(saved);
      setDraft(saved.profile);
      draftExpectedProfileRef.current = cloneSessionProfile(saved.profile);
      return saved;
    } catch (error) {
      if (gate.isCurrent(profile.id, token)) {
        setNotice({ title: "保存会话失败", message: formatError(error) });
      }
      return null;
    } finally {
      gate.finish(profile.id, token);
    }
  }

  function connectSavedDraft(saved: SessionSummary) {
    void connectSession(saved.profile.id, saved);
  }

  async function saveProfile(
    profile: SessionProfile,
    expectedProfile: SessionProfile | null,
    proxyPasswordUpdate: ProxyPasswordUpdate = null,
  ) {
    if (isBackendAvailable()) {
      return invokeBackend<SessionSummary>("save_session_profile", { profile, expectedProfile, proxyPasswordUpdate });
    }
    return createSessionSummary(profile);
  }

  async function importOpenSshConfigCandidates(candidates: OpenSshImportCandidate[]) {
    return importSessionCandidates(candidates, createOpenSshImportedProfile, (candidate) => candidate.hostAlias);
  }

  async function importPuttyConfigCandidates(candidates: PuttySessionImportCandidate[]) {
    return importSessionCandidates(candidates, createPuttyImportedProfile, (candidate) => candidate.name);
  }

  async function importShellConfigCandidates(candidates: ShellSessionImportCandidate[]) {
    return importSessionCandidates(candidates, createShellImportedProfile, (candidate) => candidate.name);
  }

  async function importSessionCandidates<C extends { id: string }>(
    candidates: C[],
    createProfile: (candidate: C) => SessionProfile,
    candidateName: (candidate: C) => string,
  ) {
    const savedIds: string[] = [];
    const failures: Array<{ id: string; message: string }> = [];
    for (const candidate of candidates) {
      try {
        const profile = prepareSessionProfile(createProfile(candidate));
        const saved = await saveProfile(profile, null);
        applySavedSession(saved, false);
        savedIds.push(candidate.id);
      } catch (error) {
        failures.push({ id: candidate.id, message: `${candidateName(candidate)}: ${formatError(error)}` });
      }
    }
    return { savedIds, failures };
  }

  function applySavedSessionState(saved: SessionSummary, activateWorkspace = true) {
    restoreTerminalInputSession(saved.profile.id);
    if (activateWorkspace) activateSession(saved.profile.id);
    sessionSummaryRefreshGateRef.current.invalidate("summaries");
    setSessions((current) => {
      const nextSessions = mergeSessionSummaries(current, saved);
      saveLocalSessionSummaries(nextSessions);
      return nextSessions;
    });
  }

  function applySavedSession(saved: SessionSummary, activateWorkspace = true) {
    keyManagerProfileMutationGateRef.current.invalidate(saved.profile.id);
    applySavedSessionState(saved, activateWorkspace);
  }

  async function connectSession(
    sessionId = activeIdRef.current,
    sessionOverride?: SessionSummary,
    activateWorkspace = true,
    interaction: ConnectionInteraction = "interactive",
  ) {
    const session = sessionOverride ?? sessionsRef.current.find((item) => item.profile.id === sessionId);
    if (!session || session.runtime.status === "connecting" || session.runtime.status === "reconnecting") return;
    if (connectionCloseGateRef.current.isActive(session.profile.id)) return;
    const attemptToken = connectionAttemptGateRef.current.begin(session.profile.id);
    if (attemptToken === null) return;

    const attemptIsCurrent = () => connectionAttemptGateRef.current.isCurrent(session.profile.id, attemptToken);
    try {
      const credentials = interaction === "silent"
        ? emptyConnectionCredentials()
        : await requestSessionCredentials(session.profile);
      if (!attemptIsCurrent() || credentials === null) return;
      let profileForConnect: SessionProfile;
      let createdConnectionSecretRefs: string[] = [];
      try {
        const persistedSecrets = await persistConnectionSecrets(
          applyConnectionCredentials(prepareSessionProfile(session.profile), credentials),
          credentials,
        );
        profileForConnect = persistedSecrets.profile;
        createdConnectionSecretRefs = persistedSecrets.createdSecretRefs;
        if (!attemptIsCurrent()) {
          await deleteUnreferencedSecrets(createdConnectionSecretRefs);
          return;
        }
      } catch (error) {
        const cleanupErrors = await deleteUnreferencedSecrets(createdConnectionSecretRefs);
        if (!attemptIsCurrent()) return;
        const message = cleanupErrors.length
          ? `${formatError(error)}；新凭据清理失败: ${cleanupErrors.join("；")}`
          : formatError(error);
        if (interaction === "interactive") setNotice({ title: "保存凭据失败", message });
        return;
      }

      const connecting = setSessionStatus({ ...session, profile: profileForConnect }, "connecting");
      sessionSummaryRefreshGateRef.current.invalidate("summaries");
      setSessions((current) => mergeSessionSummaries(current, connecting));
      if (activateWorkspace) activateSession(profileForConnect.id);

      let persistedProfileForConnect: SessionProfile | null = null;
      try {
        const persisted = await saveProfile(profileForConnect, session.profile);
        persistedProfileForConnect = persisted.profile;
        createdConnectionSecretRefs = [];
        if (!attemptIsCurrent()) return;
        applySavedSession({ ...connecting, profile: persisted.profile }, activateWorkspace);
        const credentialHandle = credentials.oneKeyId || !isBackendAvailable()
          ? null
          : await stageConnectionCredentials(invokeBackend, persisted.profile.id, credentials);
        if (!attemptIsCurrent()) return;
        const saved = isBackendAvailable()
          ? credentials.oneKeyId
            ? await invokeBackend<SessionSummary>("open_session_with_one_key", { sessionId: persisted.profile.id, oneKeyId: credentials.oneKeyId })
            : await invokeBackend<SessionSummary>("open_session", {
              request: { sessionId: persisted.profile.id, credentialHandle },
            })
          : setSessionStatus(persisted, "connected");
        if (!attemptIsCurrent()) return;
        const fallbackLog = [...(logs[persisted.profile.id] ?? []), createLocalSystemEvent(saved.profile, `PortMate: connected to ${describeProfileEndpoint(saved.profile)}`)];
        const nextLog = await callBackend("tail_log", { sessionId: persisted.profile.id, limit: 600 }, fallbackLog);
        if (!attemptIsCurrent()) return;

        replaceSessionLog(persisted.profile.id, nextLog);
        sessionSummaryRefreshGateRef.current.invalidate("summaries");
        setSessions((current) => {
          const nextSessions = mergeSessionSummaries(current, saved);
          saveLocalSessionSummaries(nextSessions);
          return nextSessions;
        });
      } catch (error) {
        const cleanupErrors = await deleteUnreferencedSecrets(createdConnectionSecretRefs);
        if (!attemptIsCurrent()) return;
        const message = cleanupErrors.length
          ? `${formatError(error)}；新凭据清理失败: ${cleanupErrors.join("；")}`
          : formatError(error);
        const failureProfile = persistedProfileForConnect ?? session.profile;
        const failed = setSessionStatus({ ...session, profile: failureProfile }, "error", message);
        const backendLog = await callBackend("tail_log", { sessionId: failureProfile.id, limit: 600 }, []);
        if (!attemptIsCurrent()) return;
        const errorText = `PortMate: connection failed: ${message}`;
        const nextLog = backendLog.length ? backendLog : [...(logs[failureProfile.id] ?? []), createLocalSystemEvent(failureProfile, errorText)];
        replaceSessionLog(failureProfile.id, nextLog);
        sessionSummaryRefreshGateRef.current.invalidate("summaries");
        setSessions((current) => mergeSessionSummaries(current, failed));
        if (interaction === "silent") {
          return;
        }
        if (isSshLikeProfile(failureProfile) && isHostKeyFailure(message)) {
          void openHostKeyPrompt(failureProfile, message, credentials);
        } else {
          setNotice({ title: "连接失败", message });
        }
      }
    } finally {
      connectionAttemptGateRef.current.finish(session.profile.id, attemptToken);
    }
  }

  async function reconnectSession(sessionId: string, activateWorkspace = true) {
    let session = sessionsRef.current.find((item) => item.profile.id === sessionId);
    if (!session) return;
    if (sessionConnectionAction(session.runtime.status) === "disconnect") {
      const disconnected = await disconnectSession(sessionId, false);
      if (!disconnected) return;
      session = disconnected;
    }
    await connectSession(sessionId, session, activateWorkspace);
  }

  async function openHostKeyPrompt(profile: SessionProfile, message: string, credentials?: ConnectionCredentials) {
    const scannedProfile = prepareSessionProfile(profile);
    const profileIsCurrent = () => {
      const current = sessionsRef.current.find((session) => session.profile.id === scannedProfile.id);
      return Boolean(current
        && hostKeyProfileSnapshotMatches(scannedProfile, prepareSessionProfile(current.profile)));
    };
    if (!profileIsCurrent()) return;
    const gate = hostKeyPromptOperationGateRef.current;
    gate.invalidate("decision");
    const token = gate.replace("scan");
    setHostKeyPrompt({ profile: scannedProfile, message, scan: null, scanError: null, busy: true });
    try {
      const credentialHandle = credentials
        ? await stageConnectionCredentials(invokeBackend, scannedProfile.id, credentials)
        : null;
      if (!gate.isCurrent("scan", token)) return;
      if (!profileIsCurrent()) {
        closeHostKeyPrompt();
        return;
      }
      const scan = await invokeBackend<HostKeyScanResult>("scan_ssh_host_key", {
        request: { profile: scannedProfile, credentialHandle },
      });
      if (!gate.isCurrent("scan", token)) return;
      if (profileIsCurrent()) {
        setHostKeyPrompt({ profile: scannedProfile, message, scan, scanError: null, busy: false });
      } else {
        closeHostKeyPrompt();
      }
    } catch (error) {
      if (!gate.isCurrent("scan", token)) return;
      if (profileIsCurrent()) {
        setHostKeyPrompt({ profile: scannedProfile, message, scan: null, scanError: formatError(error), busy: false });
      } else {
        closeHostKeyPrompt();
      }
    } finally {
      gate.finish("scan", token);
    }
  }

  async function applyHostKeyPromptDecision(decision: HostKeyDecisionValue, reconnect: boolean) {
    const prompt = hostKeyPromptRef.current;
    if (!prompt?.scan) return;
    const current = sessionsRef.current.find((session) => session.profile.id === prompt.profile.id);
    if (!current
      || !hostKeyProfileSnapshotMatches(prompt.profile, prepareSessionProfile(current.profile))) {
      closeHostKeyPrompt();
      return;
    }
    const gate = hostKeyPromptOperationGateRef.current;
    const token = gate.begin("decision");
    if (token === null) return;
    const hostKeyMutationToken = beginHostKeyMutation();
    const profile = prepareSessionProfile(prompt.profile);
    setHostKeyPrompt((current) => current ? { ...current, busy: true } : current);
    try {
      await invokeBackend<TrustedHostKey | null>("trust_scanned_host_key", {
        request: { profile, observation: prompt.scan.observation, decision },
      });
      const nextHostKeys = await callBackend("list_host_keys", {}, hostKeys);
      commitHostKeyMutation(nextHostKeys, hostKeyMutationToken);
      if (!gate.isCurrent("decision", token)) return;
      setDraft(profile);
      setHostKeyPrompt(null);
      setNotice({ title: "Host key 已确认", message: reconnect ? "已保存信任决策，正在重新连接。" : "已保存信任决策。" });
      if (reconnect) {
        void connectSession(profile.id);
      }
    } catch (error) {
      if (gate.isCurrent("decision", token)) {
        setHostKeyPrompt((current) => current ? { ...current, busy: false, scanError: formatError(error) } : current);
      }
    } finally {
      finishHostKeyMutation(hostKeyMutationToken);
      gate.finish("decision", token);
    }
  }

  function closeHostKeyPrompt() {
    hostKeyPromptOperationGateRef.current.invalidateAll();
    setHostKeyPrompt(null);
  }

  function openHostKeySettingsFromPrompt() {
    const prompt = hostKeyPromptRef.current;
    if (!prompt) return;
    const current = sessionsRef.current.find((session) => session.profile.id === prompt.profile.id);
    closeHostKeyPrompt();
    if (!current
      || !hostKeyProfileSnapshotMatches(prompt.profile, prepareSessionProfile(current.profile))) return;
    openSessionProfileDialog(current.profile, current.profile, "验证");
  }

  async function disconnectSession(sessionId = activeIdRef.current, activateWorkspace = true, reportError = true): Promise<SessionSummary | null> {
    const closeToken = beginSessionDisconnect(sessionId);
    if (closeToken === null) return null;
    const closeIsCurrent = () => connectionCloseGateRef.current.isCurrent(sessionId, closeToken);
    try {
      connectionAttemptGateRef.current.invalidate(sessionId);
      const credentialRequest = credentialResolverRef.current;
      if (credentialRequest?.sessionId === sessionId) {
        credentialResolverRef.current = null;
        setCredentialPrompt(null);
        credentialRequest.resolve(null);
      }
      const session = sessionsRef.current.find((item) => item.profile.id === sessionId);
      if (!session) return null;
      if (isBackendAvailable() && session.runtime.status === "disconnected") return session;

      const saved = isBackendAvailable()
        ? await invokeBackend<SessionSummary>("close_session", { sessionId })
        : setSessionStatus(session, "disconnected", "user closed session");
      if (!closeIsCurrent()) return null;
      const fallbackLog = [...(logs[sessionId] ?? []), createLocalSystemEvent(saved.profile, "PortMate: session disconnected")];
      const nextLog = await callBackend("tail_log", { sessionId, limit: 160 }, fallbackLog);
      if (!closeIsCurrent()) return null;

      replaceSessionLog(sessionId, nextLog);
      if (activateWorkspace) activateSession(sessionId);
      sessionSummaryRefreshGateRef.current.invalidate("summaries");
      setSessions((current) => {
        const nextSessions = mergeSessionSummaries(current, saved);
        saveLocalSessionSummaries(nextSessions);
        return nextSessions;
      });
      return saved;
    } catch (error) {
      if (!closeIsCurrent()) return null;
      if (reportError) setNotice({ title: "断开会话失败", message: formatError(error) });
      void refreshSessionSummaries();
      return null;
    } finally {
      finishSessionDisconnect(sessionId, closeToken);
    }
  }

  function routeTerminalInput(sessionId: string, text: string, origin: SyncInputOrigin = "interactive"): Promise<void> {
    const currentSessions = sessionsRef.current;
    if (!currentSessions.some((session) => session.profile.id === sessionId)) return Promise.resolve();
    const broadcastEnabled = syncInputRef.current;
    const settings = syncInputSettings;
    const paneSessionIds = workspacePaneLeaves(workspaceRootRef.current)
      .map((pane) => workspacePaneActiveView(pane).sessionId);
    const currentPaneSessions = (paneSessionIds.length ? paneSessionIds : [activeIdRef.current])
      .map((id) => currentSessions.find((session) => session.profile.id === id))
      .filter((session): session is SessionSummary => Boolean(session));
    const candidates = currentPaneSessions.map((session) => ({
      id: session.profile.id,
      kind: session.profile.kind,
      connected: session.runtime.status === "connected",
    }));
    if (!candidates.some((candidate) => candidate.id === sessionId)) {
      const source = currentSessions.find((session) => session.profile.id === sessionId);
      if (source) {
        candidates.unshift({
          id: source.profile.id,
          kind: source.profile.kind,
          connected: source.runtime.status === "connected",
        });
      }
    }
    const inputEpochs = new Map(candidates.map((candidate) => [
      candidate.id,
      captureTerminalInputEpoch(candidate.id),
    ]));
    return syncInputDispatcherRef.current.enqueue({
      sourceId: sessionId,
      text,
      broadcastEnabled,
      applyAffixes: origin !== "interactive",
      settings,
      candidates,
    }, (targetId, payload) => {
      const epoch = inputEpochs.get(targetId);
      return epoch === null || epoch === undefined
        ? Promise.resolve()
        : sendTerminalInput(targetId, payload, origin, epoch);
    }, () => syncInputRef.current).then((result) => {
      if (!result.failed.length && !result.skipped.length) return;
      const failedNames = result.failed.map((targetId) => (
        sessionsRef.current.find((session) => session.profile.id === targetId)?.profile.name ?? targetId
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

  async function sendTerminalInput(sessionId: string, text: string, origin: SyncInputOrigin, inputEpoch: number) {
    if (!sessionId || !text || !terminalInputIsCurrent(sessionId, inputEpoch)) return;
    const session = sessionsRef.current.find((item) => item.profile.id === sessionId);
    if (!session) throw new Error(`unknown session: ${sessionId}`);

    try {
      if (isBackendAvailable()) {
        if (origin === "command") {
          await invokeBackend<SessionEvent>("run_command", { sessionId, command: text });
        } else {
          await invokeBackend<SessionEvent>("send_text", { sessionId, text });
        }
        if (!terminalInputIsCurrent(sessionId, inputEpoch)) return;
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
      if (!terminalInputIsCurrent(sessionId, inputEpoch)) return;
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
    const inputEpoch = captureTerminalInputEpoch(sessionId);
    if (inputEpoch === null) return;
    await syncInputDispatcherRef.current.enqueueOperation(async () => {
      if (!terminalInputIsCurrent(sessionId, inputEpoch)) return;
      try {
        await invokeBackend<SessionEvent>("send_one_key", {
          request: {
            id: oneKeyId,
            sessionId,
            field,
            source: "prompt-completion",
            promptEventId,
          },
        });
      } catch (error) {
        if (terminalInputIsCurrent(sessionId, inputEpoch)) throw error;
      }
    });
  }

  async function sendTerminalBytes(sessionId: string, bytes: number[], inputEpoch: number) {
    if (!sessionId || !bytes.length || !terminalInputIsCurrent(sessionId, inputEpoch)) return;
    const session = sessionsRef.current.find((item) => item.profile.id === sessionId);
    if (!session) throw new Error(`unknown session: ${sessionId}`);

    try {
      if (isBackendAvailable()) {
        await invokeBackend<SessionEvent>("send_bytes", { sessionId, bytes });
        if (!terminalInputIsCurrent(sessionId, inputEpoch)) return;
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
      if (!terminalInputIsCurrent(sessionId, inputEpoch)) return;
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
    if (sendTarget === "active" && activeId !== activeIdRef.current) return;
    const currentSessions = sessionsRef.current;
    const paneSessionIds = workspacePaneLeaves(workspaceRootRef.current)
      .map((pane) => workspacePaneActiveView(pane).sessionId);
    const currentPaneSessions = paneSessionIds
      .map((id) => currentSessions.find((session) => session.profile.id === id))
      .filter((session): session is SessionSummary => Boolean(session));
    const targets = resolveSendTargets(sendTarget, activeIdRef.current, currentSessions, currentPaneSessions);
    if (!targets.length) {
      setNotice({ title: "发送", message: "没有可发送的目标会话。" });
      return;
    }
    const inputEpochs = new Map(targets.map((target) => [target, captureTerminalInputEpoch(target)]));
    const sendToken = sendOperationGateRef.current.begin("send");
    if (sendToken === null) return;
    setSendBusy(true);
    try {
      await syncInputDispatcherRef.current.enqueueOperation(async () => {
        for (let index = 0; index < Math.max(1, sendCount); index += 1) {
          await Promise.all(
            targets.map((target) => {
              const inputEpoch = inputEpochs.get(target);
              if (inputEpoch === null || inputEpoch === undefined) return Promise.resolve();
              return sendMode === "hex"
                ? sendTerminalBytes(target, bytePayload, inputEpoch)
                : sendTerminalInput(target, textPayload, "atomic", inputEpoch);
            }),
          );
          if (index + 1 < Math.max(1, sendCount)) {
            await delay(sendIntervalMs);
          }
        }
      });
      if (sendMode === "text" && textPayload.trim()) {
        rememberCommand(textPayload);
      }
    } catch (error) {
      setNotice({ title: "发送失败", message: formatError(error) });
    } finally {
      if (sendOperationGateRef.current.finish("send", sendToken)) setSendBusy(false);
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
    void import("./command-history-state").then((history) => {
      const valid = history.normalizeCommandHistoryCommand(command);
      if (!valid) return;
      const normalized = history.recordCommandHistory(
        commandHistoryEntriesRef.current,
        valid,
        commandHistoryPolicyRef.current,
      );
      commandHistoryEntriesRef.current = normalized;
      setCommandHistoryEntries(normalized);
      setCommandHistoryReady(true);
      if (!commandHistoryEnabledRef.current || !isBackendAvailable()) return;
      pendingCommandHistoryRef.current = history.queuePendingCommandHistory(
        pendingCommandHistoryRef.current,
        valid,
        commandHistoryPolicyRef.current,
      );
      if (!commandHistoryBackendReadyRef.current) return;
      enqueueCommandHistoryOperation(
        () => invokeBackend<CommandHistorySnapshot>("record_command_history", {
          command: valid,
          limit: commandHistoryPolicyRef.current.limit,
          retentionDays: commandHistoryPolicyRef.current.retentionDays,
        }),
        valid,
      );
    });
  }

  function clearCommandHistory() {
    pendingCommandHistoryRef.current = [];
    commandHistoryEntriesRef.current = [];
    setCommandHistoryEntries([]);
    try {
      window.localStorage.removeItem(COMMAND_HISTORY_STORAGE_KEY);
    } catch {
      // Clearing the in-memory list still succeeds when storage is unavailable.
    }
    if (commandHistoryBackendReadyRef.current) {
      enqueueCommandHistoryOperation(() => invokeBackend<CommandHistorySnapshot>("clear_command_history", {}));
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
    if (credentialResolverRef.current) return Promise.resolve(null);

    const ssh = profile.connection;
    const target = describeProfileEndpoint(profile) || profile.name || "SSH";
    const hasPrivateKey = ssh.identityRefs.some((identity) => Boolean(identity.path) || Boolean(identity.secretRef));
    const requestId = ++credentialRequestIdRef.current;
    const prompt: CredentialPromptState = {
      requestId,
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
      credentialResolverRef.current = { requestId, sessionId: profile.id, resolve };
      setCredentialPrompt(prompt);
    });
  }

  function completeCredentialPrompt(requestId: number, credentials: ConnectionCredentials | null) {
    const credentialRequest = credentialResolverRef.current;
    if (!credentialRequest || credentialRequest.requestId !== requestId) return;
    credentialResolverRef.current = null;
    setCredentialPrompt(null);
    credentialRequest.resolve(credentials);
  }

  async function setSerialLine(sessionId: string, line: "dtr" | "rts", value: boolean) {
    const session = sessionsRef.current.find((item) => item.profile.id === sessionId);
    if (!session || session.profile.connection.kind !== "serial") return;
    const controlToken = beginSerialControl(sessionId);
    if (controlToken === null) return;
    try {
      const saved = await invokeBackend<SessionSummary>("serial_set_lines", {
        request: { sessionId, [line]: value },
      });
      if (!serialControlOperationGateRef.current.isCurrent(sessionId, controlToken)) return;
      sessionSummaryRefreshGateRef.current.invalidate("summaries");
      setSessions((current) => mergeSessionSummaries(current, saved));
    } catch (error) {
      if (!serialControlOperationGateRef.current.isCurrent(sessionId, controlToken)) return;
      setNotice({ title: "串口控制失败", message: formatError(error) });
      void refreshSessionSummaries();
    } finally {
      finishSerialControl(sessionId, controlToken);
    }
  }

  async function sendSerialBreak(sessionId: string) {
    const session = sessionsRef.current.find((item) => item.profile.id === sessionId);
    if (!session || session.profile.connection.kind !== "serial") return;
    const controlToken = beginSerialControl(sessionId);
    if (controlToken === null) return;
    try {
      await invokeBackend("serial_send_break", { sessionId });
      if (!serialControlOperationGateRef.current.isCurrent(sessionId, controlToken)) return;
      await refreshActiveLog(sessionId);
    } catch (error) {
      if (!serialControlOperationGateRef.current.isCurrent(sessionId, controlToken)) return;
      setNotice({ title: "Break 失败", message: formatError(error) });
      void refreshActiveLog(sessionId);
    } finally {
      finishSerialControl(sessionId, controlToken);
    }
  }

  function beginSerialControl(sessionId: string) {
    const token = serialControlOperationGateRef.current.begin(sessionId);
    if (token !== null) setSerialControlBusyIds((current) => new Set(current).add(sessionId));
    return token;
  }

  function finishSerialControl(sessionId: string, token: number) {
    if (!serialControlOperationGateRef.current.finish(sessionId, token)) return;
    setSerialControlBusyIds((current) => {
      if (!current.has(sessionId)) return current;
      const next = new Set(current);
      next.delete(sessionId);
      return next;
    });
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
      return (
        <Suspense fallback={null}>
          <LazyFileManagerPanel
            active={active}
            transfers={transfers}
            dismissedTransferIds={dismissedTransferIds}
            onTransfer={(task) => updateTransfers((current) => mergeTransfers(current, task))}
            onDismissTransfer={dismissTransfer}
            onNotice={setNotice}
          />
        </Suspense>
      );
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
                busy={serialCaptureActionIds.has(active.profile.id)}
              />
            ) : null}
          />
        </Suspense>
      );
    }
    if (panel === "sysmon") {
      return (
        <Suspense fallback={null}>
          <LazySysmonSidebar
            session={active ?? null}
            enabled={workspaceDockIds.some((dock) => activeDockPanels[dock] === "sysmon")}
            onOpenDetails={() => active && setUtilityDialog("sysmon")}
          />
        </Suspense>
      );
    }
    const sendAdvancedActive = sendCount !== 1 || sendIntervalMs !== 1000 || sendTarget !== "active";
    return (
      <>
        <div className="send-toolbar" data-advanced-open={sendAdvancedOpen ? "true" : "false"}>
          <div className="send-toolbar-primary">
            <button className="send-icon-button" title="发送" aria-label="发送" onClick={() => void runSendPanel()} disabled={sendBusy}>
              <Play size={14} className="green" />
            </button>
            <label className="send-mode-label">
              <input type="radio" checked={sendMode === "text"} onChange={() => setSendMode("text")} /> 文本(T)
            </label>
            <label className="send-mode-label">
              <input type="radio" checked={sendMode === "hex"} onChange={() => setSendMode("hex")} /> Hex(H)
            </label>
            <button
              type="button"
              className="send-icon-button send-advanced-toggle"
              title={sendAdvancedActive ? "高级发送选项（已配置）" : "高级发送选项"}
              aria-label="高级发送选项"
              aria-expanded={sendAdvancedOpen}
              data-active={sendAdvancedActive ? "true" : "false"}
              onClick={() => setSendAdvancedOpen((current) => !current)}
            >
              <SlidersHorizontal size={14} />
            </button>
            {syncInput ? <span className="sync-badge">同步输入 · {syncInputTargetCount} 目标</span> : null}
          </div>
          {sendAdvancedOpen ? (
            <div className="send-advanced-controls" role="group" aria-label="高级发送选项">
              <label>
                <span>计数</span>
                <input type="number" min={1} className="number-input" aria-label="发送次数" value={sendCount} onChange={(event) => setSendCount(Math.max(1, Number(event.target.value) || 1))} />
              </label>
              <label>
                <span>间隔</span>
                <input type="number" min={0} className="number-input" aria-label="发送间隔（毫秒）" value={sendIntervalMs} onChange={(event) => setSendIntervalMs(Math.max(0, Number(event.target.value) || 0))} />
              </label>
              <label>
                <span>目标</span>
                <select className="target-input" aria-label="发送目标" value={sendTarget} onChange={(event) => setSendTarget(event.target.value as SendTarget)}>
                  <option value="active">当前会话</option>
                  <option value="panes">打开窗格</option>
                  <option value="connected">全部已连接</option>
                </select>
              </label>
            </div>
          ) : null}
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
                      const disabled = menuItemDisabled(item, menuCapabilityContext)
                        || (disconnectingSessionIds.has(activeId) && (item === "启动会话" || item === "关闭会话"));
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
            connectionBusyIds={disconnectingSessionIds}
            serialControlBusyIds={serialControlBusyIds}
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
          title="切换 Insert/Normal 模式 (Esc / i)"
          aria-label={`当前${terminalKeyModeLabel(activeTerminalKeyMode)}，切换 Insert/Normal 模式`}
          onClick={() => setActiveWorkspaceViewKeyMode(toggleTerminalInsertNormalMode(activeTerminalKeyMode))}
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
            connectionBusy={disconnectingSessionIds.has(contextMenu.sessionId ?? activeId)}
            profileBusy={profileShortcutBusyIds.has(contextMenu.sessionId ?? activeId)}
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
            exportBusy={terminalExportBusyViewIds.has(contextMenu.viewId)}
            onAction={(action) => handleTerminalContextMenuAction(action, contextMenu)}
          />
        </Suspense>
      )}

      {workspaceViewContextMenu && workspaceContextPane && workspaceContextView && workspaceContextSession && (
        <Suspense fallback={null}>
          <LazyWorkspaceViewContextMenu
            state={workspaceViewContextMenu}
            view={workspaceContextView}
            sessionStatus={workspaceContextSession.runtime.status}
            connectionBusy={disconnectingSessionIds.has(workspaceContextSession.profile.id)}
            profileBusy={profileShortcutBusyIds.has(workspaceContextSession.profile.id)}
            exportBusy={terminalExportBusyViewIds.has(workspaceContextView.id)}
            label={workspaceContextView.title || workspaceContextSession.profile.name}
            colors={tabColorChoices}
            canMerge={workspaceContextCanMerge}
            canSwap={workspaceContextCanSwap}
            canZoom={workspacePanes.length > 1}
            {...workspaceContextCapabilities}
            canDetach={workspaceContextCapabilities.canDetach && !detachingWorkspaceViewIds.has(workspaceContextView.id)}
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
        <Suspense fallback={null}>
          <LazyTerminalSettingsDialog
            initialPrefs={terminalPrefs}
            normalizePrefs={normalizeTerminalPrefs}
            sessions={sessions}
            syncSettings={syncInputSettings}
            workspaceKeymap={workspaceKeymap}
            onPrefsChange={setTerminalPrefs}
            onClearCommandHistory={clearCommandHistory}
            onSyncSettingsChange={setSyncInputSettings}
            onWorkspaceKeymapChange={setWorkspaceKeymap}
            onClose={() => setDialog(null)}
          />
        </Suspense>
      )}
      {dialog === "session" && (
        <Suspense fallback={null}>
          <LazySessionSettingsDialog
            draft={draft}
            mode={sessionSettingsMode}
            prepareProfile={prepareSessionProfile}
            serialPorts={serialPorts}
            initialSection={sessionSettingsSection}
            onDraftChange={setDraft}
            onSave={saveDraft}
            onConnect={connectSavedDraft}
            onClose={() => {
              draftExpectedProfileRef.current = null;
              setDialog(null);
            }}
          />
        </Suspense>
      )}
      {utilityDialog === "session-import" && (
        <Suspense fallback={null}>
          <LazySessionImportDialog
            onImportOpenSsh={importOpenSshConfigCandidates}
            onImportPutty={importPuttyConfigCandidates}
            onImportShell={importShellConfigCandidates}
            onClose={() => setUtilityDialog(null)}
          />
        </Suspense>
      )}
      {utilityDialog === "transfer" && active && (
        <Suspense fallback={null}>
          <LazyTransferDialog
            key={active.profile.id}
            session={active}
            transfers={transfers}
            dismissedTransferIds={dismissedTransferIds}
            onClose={() => setUtilityDialog(null)}
            onTask={(task) => {
              updateTransfers((current) => mergeTransfers(current, task));
            }}
            onDismissTransfer={dismissTransfer}
            onNotice={(message) => {
              setNotice({ title: "传输任务", message });
            }}
          />
        </Suspense>
      )}
      {utilityDialog === "tunnel" && active && (
        <Suspense fallback={null}>
          <LazyTunnelDialog key={active.profile.id} session={active} onClose={() => setUtilityDialog(null)} onDone={(label) => {
            setUtilityDialog(null);
            setNotice({ title: "端口转发", message: label });
          }} />
        </Suspense>
      )}
      {utilityDialog === "tmux" && active && (
        <Suspense fallback={null}>
          <LazyTmuxDialog key={active.profile.id} session={active} onClose={() => setUtilityDialog(null)} onDone={(message) => {
            setUtilityDialog(null);
            setNotice({ title: "Tmux", message });
            void refreshActiveLog(active.profile.id);
          }} />
        </Suspense>
      )}
      {utilityDialog === "sysmon" && active && (
        <Suspense fallback={null}>
          <LazySysmonDialog key={active.profile.id} session={active} onClose={() => setUtilityDialog(null)} />
        </Suspense>
      )}
      {utilityDialog === "search" && (
        <Suspense fallback={null}>
          <LazySearchDialog state={searchDialog} sessions={sessions} logs={logs} onChange={setSearchDialog} onSelect={(sessionId) => {
            activateSession(sessionId);
            setUtilityDialog(null);
          }} onClose={() => setUtilityDialog(null)} />
        </Suspense>
      )}
      {utilityDialog === "logs" && (
        <Suspense fallback={null}>
          <LazyLogManagerDialog sessions={sessions} activeId={activeId} onClose={() => setUtilityDialog(null)} onNotice={(message) => setNotice({ title: "日志管理", message })} />
        </Suspense>
      )}
      {utilityDialog === "keys" && (
        <Suspense fallback={null}>
          <LazyKeyManagerDialog
            hostKeys={hostKeys}
            sessions={sessions}
            prepareProfile={prepareSessionProfile}
            onHostKeyMutationStart={beginHostKeyMutation}
            onChange={commitHostKeyMutation}
            onHostKeyMutationFinish={finishHostKeyMutation}
            onProfileMutationStart={beginKeyManagerProfileMutation}
            onProfileChange={commitKeyManagerProfileMutation}
            onProfileMutationCurrent={isKeyManagerProfileMutationCurrent}
            onProfileMutationFinish={finishKeyManagerProfileMutation}
            credentialOperationBusy={keyManagerCredentialOperationToken !== null}
            credentialSyncRevision={keyManagerCredentialSyncRevision}
            onCredentialOperationStart={beginKeyManagerCredentialOperation}
            onCredentialOperationFinish={finishKeyManagerCredentialOperation}
            onClose={() => setUtilityDialog(null)}
          />
        </Suspense>
      )}
      {utilityDialog === "mcp" && (
        <Suspense fallback={null}>
          <LazyMcpDialog
            grants={grants}
            audit={audit}
            sessions={sessions}
            onClose={() => setUtilityDialog(null)}
            onGrantMutationStart={beginGrantMutation}
            onGrantChange={commitGrantMutation}
            onGrantMutationFinish={finishGrantMutation}
            onAuditChange={updateAudit}
          />
        </Suspense>
      )}
      {utilityDialog === "one-keys" && (
        <Suspense fallback={null}>
          <LazyOneKeyDialog
            oneKeys={oneKeys}
            sessions={sessions}
            activeId={activeId}
            onMutationStart={beginOneKeyMutation}
            onChange={commitOneKeyMutation}
            onMutationFinish={finishOneKeyMutation}
            onClose={() => setUtilityDialog(null)}
          />
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
      {utilityDialog === "custom-scripts" && (
        <Suspense fallback={null}>
          <LazyCustomScriptDialog
            sessions={sessions}
            activeId={activeId}
            onNotice={(message) => setNotice({ title: "自定义脚本", message })}
            onClose={() => setUtilityDialog(null)}
          />
        </Suspense>
      )}
      {credentialPrompt && (
        <Suspense fallback={null}>
          <LazyCredentialDialog
            key={credentialPrompt.requestId}
            request={credentialPrompt}
            onCancel={() => completeCredentialPrompt(credentialPrompt.requestId, null)}
            onSubmit={(credentials) => completeCredentialPrompt(credentialPrompt.requestId, credentials)}
          />
        </Suspense>
      )}
      {hostKeyPrompt && (
        <HostKeyConfirmDialog
          state={hostKeyPrompt}
          onDecision={(decision, reconnect) => void applyHostKeyPromptDecision(decision, reconnect)}
          onOpenSettings={openHostKeySettingsFromPrompt}
          onClose={closeHostKeyPrompt}
        />
      )}
      {notice && (
        <Suspense fallback={null}>
          <LazyNoticeDialog title={notice.title} message={notice.message} link={notice.link} onClose={() => setNotice(null)} />
        </Suspense>
      )}
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
  const submitPendingRef = useRef(false);
  const overlayRef = useRef<HTMLDivElement | null>(null);
  const primaryRef = useRef<HTMLInputElement | HTMLButtonElement | null>(null);

  useEffect(() => {
    setPassword("");
    setError("");
    window.requestAnimationFrame(() => primaryRef.current?.focus({ preventScroll: true }));
  }, [state.mode]);

  async function submit(event?: FormEvent) {
    event?.preventDefault();
    if (submitPendingRef.current || busy || state.mode === "preparing" || state.mode === "error") return;
    submitPendingRef.current = true;
    setBusy(true);
    setError("");
    try {
      await onUnlock(password);
    } catch (unlockError) {
      setPassword("");
      setError(formatError(unlockError));
      window.requestAnimationFrame(() => primaryRef.current?.focus({ preventScroll: true }));
    } finally {
      submitPendingRef.current = false;
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

  function handleDockTabKey(event: React.KeyboardEvent<HTMLButtonElement>, index: number) {
    let nextIndex = index;
    if (event.key === "ArrowLeft") nextIndex = (index - 1 + panels.length) % panels.length;
    else if (event.key === "ArrowRight") nextIndex = (index + 1) % panels.length;
    else if (event.key === "Home") nextIndex = 0;
    else if (event.key === "End") nextIndex = panels.length - 1;
    else return;

    event.preventDefault();
    onActivate(panels[nextIndex]);
    const tabs = event.currentTarget.closest(".workspace-dock-tabs")?.querySelectorAll<HTMLButtonElement>(".workspace-dock-tab-label");
    tabs?.[nextIndex]?.focus();
  }

  return (
    <aside className={`workspace-dock workspace-dock-${dock}`} data-dock={dock} data-active-panel={activePanel}>
      <div
        className="workspace-dock-tabs"
        role="tablist"
        aria-label={`${workspaceDockMeta[dock].label}停靠区工具`}
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
          const tabId = `workspace-dock-${dock}-tab-${panel}`;
          const panelId = `workspace-dock-${dock}-panel-${panel}`;
          return (
            <header
              key={panel}
              className={active ? "workspace-dock-tab active" : "workspace-dock-tab"}
              data-panel={panel}
              draggable
              onDragStart={(event) => onDragStart(event, panel)}
              onDragEnd={(event) => {
                delete event.currentTarget.dataset.panelDropPosition;
                onDragEnd();
              }}
              onDragOver={(event) => {
                event.stopPropagation();
                onDragOver(event);
                const bounds = event.currentTarget.getBoundingClientRect();
                const after = event.clientX >= bounds.left + bounds.width / 2;
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
                id={tabId}
                type="button"
                className="workspace-dock-tab-label"
                role="tab"
                aria-selected={active}
                aria-controls={panelId}
                tabIndex={active ? 0 : -1}
                title={`聚焦${metadata.label}`}
                onClick={() => onActivate(panel)}
                onKeyDown={(event) => handleDockTabKey(event, index)}
              >
                <PanelIcon size={13} />
                <span>{metadata.label}</span>
              </button>
              <button
                type="button"
                className="workspace-dock-tab-close"
                title={`隐藏${metadata.label}`}
                aria-label={`隐藏${metadata.label}`}
                onClick={(event) => {
                  event.stopPropagation();
                  onClose(panel);
                }}
              >
                <X size={12} />
              </button>
            </header>
          );
        })}
      </div>
      <div
        className="workspace-dock-panels"
        onDragOver={(event) => {
          if (event.target === event.currentTarget) onDragOver(event);
        }}
        onDrop={(event) => {
          if (event.target === event.currentTarget) onDrop(event, dock, panels.length);
        }}
      >
        {panels.map((panel) => {
          const active = panel === activePanel;
          const tabId = `workspace-dock-${dock}-tab-${panel}`;
          const panelId = `workspace-dock-${dock}-panel-${panel}`;
          return (
            <section
              key={panel}
              id={panelId}
              className={`workspace-dock-content panel-${panel}`}
              data-panel={panel}
              role="tabpanel"
              aria-labelledby={tabId}
              hidden={!active}
            >
              {renderPanel(panel)}
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
  busy,
}: {
  frames: SerialCaptureFrame[];
  onOpen: () => void;
  onClear: () => void;
  onExport: (frameIds: string[]) => void;
  canExport: boolean;
  busy: boolean;
}) {
  const [direction, setDirection] = useState<SerialCaptureDirectionFilter>("all");
  const [query, setQuery] = useState("");
  const visible = useMemo(
    () => filterSerialCaptureFrames(frames, direction, query),
    [frames, direction, query],
  );

  return (
    <div className="serial-monitor" aria-busy={busy}>
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
            disabled={busy || !canExport || !visible.length}
            onClick={() => onExport(visible.map((frame) => frame.id))}
          >
            <Download size={13} />
          </button>
          <button type="button" title="清空串口捕获" aria-label="清空串口捕获" disabled={busy || !frames.length} onClick={onClear}>
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
  connectionBusyIds,
  serialControlBusyIds,
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
  connectionBusyIds: ReadonlySet<string>;
  serialControlBusyIds: ReadonlySet<string>;
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
        connectionBusyIds={connectionBusyIds}
        serialControlBusyIds={serialControlBusyIds}
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
  connectionBusyIds: ReadonlySet<string>;
  serialControlBusyIds: ReadonlySet<string>;
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
  const connectionBusy = Boolean(session && props.connectionBusyIds.has(session.profile.id));
  const serialControlBusy = Boolean(session && props.serialControlBusyIds.has(session.profile.id));
  const connectionActionLabel = connectionBusy ? "正在断开" : connectionAction === "disconnect" ? "断开" : "连接";
  const connectionHealth = session ? sessionRuntimeHealthDescription(session.runtime) : "";
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
            const health = sessionRuntimeHealthDescription(item.runtime);
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
                  aria-label={label}
                  aria-description={health}
                  className="workspace-pane-tab-label"
                  title={`${label}\n${health}`}
                  onMouseDown={(event) => event.stopPropagation()}
                  onDoubleClick={(event) => {
                    event.stopPropagation();
                    props.onRenameView(node.id, view.id);
                  }}
                  onAuxClick={(event) => {
                    if (event.button !== 1) return;
                    event.preventDefault();
                    event.stopPropagation();
                    props.onRenameView(node.id, view.id);
                  }}
                  onClick={(event) => {
                    event.stopPropagation();
                    props.onActivate(node.id, view.id);
                  }}
                >
                  <span className="session-status-dot" aria-hidden="true" />
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
          <div className="pane-serial-tools" aria-label="串口线路控制" aria-busy={serialControlBusy}>
            <button
              type="button"
              className={serialConnection.dtr ? "active" : ""}
              aria-pressed={serialConnection.dtr}
              aria-busy={serialControlBusy}
              disabled={serialControlBusy}
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
              aria-busy={serialControlBusy}
              disabled={serialControlBusy}
              title="切换 RTS"
              onClick={(event) => {
                event.stopPropagation();
                props.onSetSerialLine(session.profile.id, "rts", !serialConnection.rts);
              }}
            >RTS</button>
            <button
              type="button"
              title="发送 Break"
              aria-busy={serialControlBusy}
              disabled={serialControlBusy}
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
            title={`${connectionActionLabel} ${session.profile.name}\n${connectionHealth}`}
            aria-label={`${connectionActionLabel} ${session.profile.name}`}
            aria-description={connectionHealth}
            aria-busy={connectionBusy}
            disabled={connectionBusy}
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
  if (hasActiveInteractionLayer()) return false;
  const element = target instanceof Element ? target : document.activeElement;
  const pane = element?.closest(".terminal-pane");
  if (pane && !pane.classList.contains("active")) return false;
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
      setBusy(false);
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
      setBusy(false);
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



function DialogFrame({
  title,
  className,
  onClose,
  closeDisabled = false,
  children,
}: {
  title: string;
  className: string;
  onClose: () => void;
  closeDisabled?: boolean;
  children: React.ReactNode;
}) {
  return (
    <div className="dialog-backdrop">
      <section className={`wind-dialog ${className}`}>
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>{title}</strong>
          <button onClick={onClose} disabled={closeDisabled}><X size={22} /></button>
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


function createTerminalPrefs() {
  return {
    startupMode: "last",
    startupSessions: ["", "", "", ""],
    terminalTextExportDirectory: "",
    lockOnIdle: false,
    lockScreenTimeoutMinutes: 30,
    requireMasterPassword: false,
    completionEnabled: true,
    oneKeyCompletionEnabled: true,
    semanticHighlightingEnabled: true,
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
      fontFamily: DEFAULT_TERMINAL_FONT_FAMILY,
      fontSize: 13,
      theme: "portmate-dark",
      backgroundOpacity: 100,
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
    transfer: { sftp: true, scp: true, tftp: true, xmodem: true, ymodem: true, zmodem: true, rateLimitBytesPerSecond: null, defaultLocalDir: null },
  };
}

function createOpenSshImportedProfile(candidate: OpenSshImportCandidate): SessionProfile {
  const profile = createSessionDraft();
  return {
    ...profile,
    name: candidate.hostAlias,
    kind: "ssh",
    connection: createOpenSshImportConnection(candidate),
  };
}

function createPuttyImportedProfile(candidate: PuttySessionImportCandidate): SessionProfile {
  const profile = createSessionDraft();
  return {
    ...profile,
    name: candidate.name,
    kind: candidate.kind,
    connection: createPuttyImportConnection(candidate),
    terminal: applyPuttyImportTerminal(profile.terminal, candidate),
  };
}

function createShellImportedProfile(candidate: ShellSessionImportCandidate): SessionProfile {
  const profile = createSessionDraft();
  return {
    ...profile,
    name: candidate.name,
    kind: "shell",
    connection: createShellImportConnection(candidate),
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

function defaultSessionName(profile: SessionProfile) {
  const connection = profile.connection;
  switch (connection.kind) {
    case "shell":
      return connection.program || "Shell";
    case "ssh":
    case "tmux":
      return formatSshTarget(connection) || "SSH";
    case "telnet":
    case "tcp":
      return formatTcpConnectionTarget(connection.kind, connection)
        || (connection.kind === "telnet" ? "Telnet" : "Tcp");
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

function setSessionStatus(session: SessionSummary, status: SessionStatus, reason?: string): SessionSummary {
  const now = new Date().toISOString();
  return {
    ...session,
    runtime: {
      ...transitionSessionRuntimeStatus(session.runtime, status, now, reason),
      title: session.profile.name,
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

function emptyConnectionCredentials(): ConnectionCredentials {
  return {
    username: null,
    password: null,
    passphrase: null,
    oneKeyId: null,
    savePassword: false,
    savePassphrase: false,
  };
}

async function persistConnectionSecrets(
  profile: SessionProfile,
  credentials: ConnectionCredentials,
): Promise<{ profile: SessionProfile; createdSecretRefs: string[] }> {
  if (!isBackendAvailable() || !isSshLikeProfile(profile)) {
    return { profile, createdSecretRefs: [] };
  }
  let connection = profile.connection;
  const createdSecretRefs: string[] = [];
  try {
    if (credentials.savePassword && credentials.password) {
      const response = await invokeBackend<{ secretRef: string }>("save_secret", {
        request: { secretRef: null, secret: credentials.password, storage: "portable" },
      });
      createdSecretRefs.push(response.secretRef);
      connection = { ...connection, passwordSecretRef: response.secretRef };
    }
    if (credentials.savePassphrase && credentials.passphrase) {
      const response = await invokeBackend<{ secretRef: string }>("save_secret", {
        request: { secretRef: null, secret: credentials.passphrase, storage: "portable" },
      });
      createdSecretRefs.push(response.secretRef);
      connection = { ...connection, passphraseSecretRef: response.secretRef };
    }
  } catch (error) {
    const cleanupErrors = await deleteUnreferencedSecrets(createdSecretRefs);
    if (cleanupErrors.length) {
      throw new Error(`${formatError(error)}；已写入凭据清理失败: ${cleanupErrors.join("；")}`);
    }
    throw error;
  }
  return { profile: { ...profile, connection }, createdSecretRefs };
}

async function deleteUnreferencedSecrets(secretRefs: readonly string[]): Promise<string[]> {
  const errors: string[] = [];
  for (const secretRef of [...new Set(secretRefs)].reverse()) {
    try {
      await invokeBackend("delete_secret", { secretRef });
    } catch (error) {
      errors.push(formatError(error));
    }
  }
  return errors;
}


function isSshLikeProfile(profile: SessionProfile): profile is SessionProfile & { connection: Extract<ConnectionConfig, { kind: "ssh" | "tmux" }> } {
  return profile.connection.kind === "ssh" || profile.connection.kind === "tmux";
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
  return JSON.stringify(sessions);
}

function cloneSessionSummaries(sessions: readonly SessionSummary[]): SessionSummary[] {
  if (typeof structuredClone === "function") return structuredClone(sessions) as SessionSummary[];
  return JSON.parse(JSON.stringify(sessions)) as SessionSummary[];
}

function mergeSessionSummaries(current: SessionSummary[], saved: SessionSummary) {
  const index = current.findIndex((session) => session.profile.id === saved.profile.id);
  if (index < 0) return [...current, saved];
  return current.map((session, itemIndex) => itemIndex === index ? saved : session);
}

function saveLocalSessionSummaries(sessions: SessionSummary[]) {
  try {
    window.localStorage.setItem("portmate.sessions", JSON.stringify({ version: 1, sessions }));
  } catch {
    // This cache is only a detached-window fallback.
  }
}

async function openWorkspaceWindow(windowId: string): Promise<void> {
  const path = buildWorkspaceWindowPath({ windowId });
  if (!isBackendAvailable()) {
    const popup = window.open(
      path,
      windowId,
      `popup,width=${WORKSPACE_WINDOW_WIDTH},height=${WORKSPACE_WINDOW_HEIGHT},resizable=yes`,
    );
    if (!popup) throw new Error("浏览器阻止了工作区窗口，请允许 PortMate 打开弹出窗口。");
    popup.focus();
    return;
  }
  const child = new WebviewWindow(windowId, {
    url: path,
    title: "PortMate",
    center: true,
    visible: false,
    width: WORKSPACE_WINDOW_WIDTH,
    height: WORKSPACE_WINDOW_HEIGHT,
    minWidth: WORKSPACE_WINDOW_MIN_WIDTH,
    minHeight: WORKSPACE_WINDOW_MIN_HEIGHT,
    preventOverflow: true,
  });
  await waitForChildWindowReady(child, async () => {
    await placeAndTrackChildWindow(child, {
      storageKey: null,
      width: WORKSPACE_WINDOW_WIDTH,
      height: WORKSPACE_WINDOW_HEIGHT,
      minWidth: WORKSPACE_WINDOW_MIN_WIDTH,
      minHeight: WORKSPACE_WINDOW_MIN_HEIGHT,
    });
    void child.setFocus().catch(() => {});
  }, "创建工作区窗口超时");
}

type DetachedPaneWindowController = {
  close: () => Promise<void>;
};

async function openDetachedPaneWindow(
  request: DetachedPaneRequest,
  sessionName: string,
): Promise<DetachedPaneWindowController> {
  const path = buildDetachedPanePath(request);
  if (!isBackendAvailable()) {
    const popup = window.open(path, request.windowId, "popup,width=960,height=680,resizable=yes");
    if (!popup) throw new Error("浏览器阻止了独立窗口，请允许 PortMate 打开弹出窗口。");
    return { close: async () => popup.close() };
  }
  const child = new WebviewWindow(request.windowId, {
    url: path,
    title: `${sessionName} - PortMate`,
    center: true,
    visible: false,
    width: 960,
    height: 680,
    minWidth: 640,
    minHeight: 400,
    preventOverflow: true,
  });
  await waitForChildWindowReady(child, () => placeAndTrackChildWindow(child, {
    storageKey: detachedPaneWindowGeometryKey(request.viewId),
    width: 960,
    height: 680,
    minWidth: 640,
    minHeight: 400,
  }), "创建独立窗口超时");
  return { close: () => child.destroy() };
}

function loadWorkspaceSnapshot(storageKey: string | null = WORKSPACE_STORAGE_KEY): WorkspaceSnapshot {
  const stored = storageKey ? loadLocalValue<unknown>(storageKey, null) : null;
  if (stored) return resetWorkspaceTerminalKeyModes(sanitizeWorkspaceSnapshot(stored));
  if (!storageKey) return { ...emptyWorkspaceSnapshot };
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
    terminalTextExportDirectory: normalizeTerminalExportDirectory(source.terminalTextExportDirectory),
    lockOnIdle: booleanPreference(source.lockOnIdle, defaults.lockOnIdle),
    lockScreenTimeoutMinutes: normalizeScreenLockTimeoutMinutes(source.lockScreenTimeoutMinutes),
    requireMasterPassword: booleanPreference(source.requireMasterPassword, defaults.requireMasterPassword),
    completionEnabled: booleanPreference(source.completionEnabled, defaults.completionEnabled),
    oneKeyCompletionEnabled: booleanPreference(source.oneKeyCompletionEnabled, defaults.oneKeyCompletionEnabled),
    semanticHighlightingEnabled: booleanPreference(source.semanticHighlightingEnabled, defaults.semanticHighlightingEnabled),
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
  const stored = readStoredScreenLockState();
  return stored ?? (lockOnStartup ? createStartupScreenLockState() : null);
}

function readStoredScreenLockState(fallbackLockedAt = Date.now()): ScreenLockState | null | undefined {
  try {
    const raw = window.localStorage.getItem(SCREEN_LOCK_STORAGE_KEY);
    const decoded = decodeStoredScreenLockMarker(raw, fallbackLockedAt);
    if (!decoded) return null;
    return {
      reason: "restored",
      lockedAt: decoded.marker.lockedAt,
      mode: "preparing",
      restoreVaultLocked: loadScreenLockVaultRestoreState(),
      repairMarker: decoded.recovered,
      message: "",
    };
  } catch {
    return undefined;
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
  try {
    window.localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Local preferences remain available in memory when browser storage is unavailable.
  }
}

function cloneSessionProfile(profile: SessionProfile): SessionProfile {
  return JSON.parse(JSON.stringify(profile)) as SessionProfile;
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

function formatDateTime(value?: string | null) {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
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
