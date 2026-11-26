import ShellElevationIndicator from "./ShellElevationIndicator";
import { shellElevationRequested } from "./shell-elevation";
import { t, useLocale } from "./i18n";
import { useEffect, useRef, useState } from "react";
import type { MouseEvent as ReactMouseEvent, SetStateAction } from "react";
import { emitTo, listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { PanelLeftOpen, Play, RefreshCw, Square } from "lucide-react";
import { invokeBackend, isBackendAvailable } from "./api";
import { AsyncOperationQueue } from "./async-operation-queue";
import type { CommandSubmissionSource } from "./command-submission";
import ChildWindowScreenLockOverlay from "./ChildWindowScreenLockOverlay";
import PortMateIcon from "./PortMateIcon";
import { TerminalContextMenu, type TerminalContextAction } from "./ContextMenus";
import { hasActiveModalLayer, INTERACTION_LAYER_DISMISS_EVENT } from "./modal-interaction-boundary";
import { pasteClipboardIntoTerminalView } from "./terminal-clipboard";
import { downloadTerminalText } from "./terminal-export-download";
import { requestTerminalTextExport, type TerminalTextExportSource } from "./terminal-export-event";
import { chooseTerminalTextExportPath, normalizeTerminalExportDirectory, terminalTextExportFileName, terminalTextExportRequest } from "./terminal-export-path";
import { requestTerminalSearch } from "./terminal-search";
import {
  COMMAND_HISTORY_STORAGE_KEY,
  commandHistoryCommands,
  commandHistorySnapshot,
  normalizeCommandHistory,
  normalizeCommandHistoryPolicy,
  recordCommandHistory,
} from "./command-history-state";
import {
  buildDetachedPanePath,
  DETACHED_PANE_EVENT,
  DETACHED_PANE_MESSAGE_TYPE,
  DETACHED_PANE_RESULT_EVENT,
  normalizeDetachedPaneResult,
  normalizeDetachedPaneResultMessage,
  SESSION_PROFILE_DELETED_EVENT,
  SESSION_PROFILE_UPDATED_EVENT,
  upsertDetachedSessionSummary,
} from "./detached-pane-state";
import type { DetachedPaneCommand, DetachedPaneRequest, DetachedPaneResult } from "./detached-pane-state";
import { decodeStoredScreenLockMarker, isScreenLockShortcut, SCREEN_LOCK_STORAGE_KEY } from "./screen-lock-state";
import type { ScreenLockMarker } from "./screen-lock-state";
import { sessionConnectionAction, sessionRuntimeHealthDescription } from "./session-runtime-state";
import { readSessionSummaryCache, SESSION_SUMMARY_CACHE_STORAGE_KEY } from "./session-summary-cache";
import type { SyncInputOrigin } from "./sync-input-state";
import TerminalCanvas from "./TerminalCanvas";
import type { TerminalInputSendOptions } from "./terminal-input-pump";
import { TerminalSessionInput } from "./terminal-session-input";
import { normalizeQuickCommandLibrary, QUICK_COMMAND_STORAGE_KEY } from "./quick-command-state";
import type { OneKeyPromptField } from "./one-key-completion-state";
import type { CommandHistorySnapshot, DeleteSessionProfileResponse, OneKeySummary, SessionEvent, SessionSummary } from "./types";
import { terminalKeyModeLabel, useTerminalKeyModeShortcutHint, toggleTerminalInsertNormalMode } from "./terminal-key-mode";
import type { TerminalKeyMode } from "./terminal-key-mode";

type DetachedOwnerControlAction = Exclude<DetachedPaneCommand["action"], "lock-screen">;
const DETACHED_REATTACH_RESULT_TIMEOUT_MS = 5_000;

type DetachedTerminalInteractionPrefs = {
  oneKeyCompletionEnabled: boolean;
  completionSettings: Record<string, unknown>;
  completionHistory: string[];
  completionQuickCommands: ReturnType<typeof readCompletionQuickCommands>;
  historyEnabled: boolean;
  mouseReporting: boolean;
  copyOnSelect: boolean;
};

export default function DetachedPaneApp({ request }: { request: DetachedPaneRequest }) {
  useLocale();
  const [sessions, setSessionsState] = useState<SessionSummary[]>(loadLocalSessions);
  const sessionsRef = useRef(sessions);
  const [events, setEvents] = useState<SessionEvent[]>([]);
  const [oneKeys, setOneKeys] = useState<OneKeySummary[]>([]);
  const [terminalInteractionPrefs, setTerminalInteractionPrefs] = useState(() => readTerminalInteractionPrefs(request.sessionId));
  const [keyMode, setKeyMode] = useState<TerminalKeyMode>(request.keyMode);
  const [viewTitle, setViewTitle] = useState(request.title);
  const [error, setError] = useState("");
  const errorRef = useRef(error);
  const [screenLock, setScreenLock] = useState<ScreenLockMarker | null>(readScreenLockMarker);
  const [ownerCommandBusy, setOwnerCommandBusy] = useState<DetachedOwnerControlAction | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [exportBusy, setExportBusy] = useState(false);
  const [contextMenu, setContextMenu] = useState<{
    x: number;
    y: number;
    alternate: boolean;
    hasSelection: boolean;
  } | null>(null);
  const refreshNowRef = useRef<(() => Promise<void>) | null>(null);
  const sessionRefreshGenerationRef = useRef(0);
  const ownerCommandBusyRef = useRef<DetachedOwnerControlAction | null>(null);
  const commandHistoryOperationRef = useRef<Promise<void>>(Promise.resolve());
  const inputQueueRef = useRef(new AsyncOperationQueue());
  const terminalInputRef = useRef<TerminalSessionInput | null>(null);
  errorRef.current = error;
  const session = sessions.find((item) => item.profile.id === request.sessionId);
  if (!terminalInputRef.current) {
    terminalInputRef.current = new TerminalSessionInput(invokeBackend, session, {
      onError: (inputError) => setError(formatDetachedError(inputError)),
      onSuccess: () => { if (errorRef.current) setError(""); },
    });
  }
  const keyModeShortcutHint = useTerminalKeyModeShortcutHint(session?.profile.connection.kind === "serial", request.viewId);
  const connectionAction = session ? sessionConnectionAction(session.runtime.status) : "connect";
  const runtimeHealth = session ? sessionRuntimeHealthDescription(session.runtime) : t("session-unavailable");
  const statusText = error || runtimeHealth;
  const statusError = Boolean(error) || session?.runtime.status === "blocked" || session?.runtime.status === "error";

  function setSessions(update: SetStateAction<SessionSummary[]>) {
    const next = typeof update === "function" ? update(sessionsRef.current) : update;
    const previous = sessionsRef.current.find(item => item.profile.id === request.sessionId);
    const current = next.find(item => item.profile.id === request.sessionId);
    if (previous && current && previous.profile.name !== current.profile.name) setViewTitle("");
    sessionsRef.current = next;
    // Fence before React commits: a pending IPC can settle between the event
    // callback and the next render and must not drain the old connection's lane.
    terminalInputRef.current!.update(next.find((item) => item.profile.id === request.sessionId));
    setSessionsState(next);
  }

  useEffect(() => {
    terminalInputRef.current?.update(sessionsRef.current.find((item) => item.profile.id === request.sessionId));
    return () => terminalInputRef.current?.update();
  }, [request.sessionId]);

  useEffect(() => {
    document.title = session ? `${viewTitle || session.profile.name} - PortMate` : "PortMate Detached Pane";
  }, [viewTitle, session?.profile.name]);

  useEffect(() => {
    if (!isBackendAvailable()) return;
    const policy = normalizeCommandHistoryPolicy(
      terminalInteractionPrefs.completionSettings.historyLimit,
      terminalInteractionPrefs.completionSettings.historyRetentionDays,
    );
    void invokeBackend("configure_command_history", {
      enabled: terminalInteractionPrefs.historyEnabled,
      limit: policy.limit,
      retentionDays: policy.retentionDays,
    }).catch((error) => {
      console.warn("PortMate detached command history policy synchronization failed", error);
    });
  }, [request.sessionId, terminalInteractionPrefs.historyEnabled,
    terminalInteractionPrefs.completionSettings.historyLimit,
    terminalInteractionPrefs.completionSettings.historyRetentionDays]);

  useEffect(() => {
    window.history.replaceState(null, "", buildDetachedPanePath({ ...request, title: viewTitle, keyMode }));
  }, [keyMode, request, viewTitle]);

  useEffect(() => {
    let disposed = false;
    let eventSignature = "";
    const sessionsPoll = shareRefresh(async () => {
      const generation = sessionRefreshGenerationRef.current;
      const nextSessions = isBackendAvailable() ? await invokeBackend<SessionSummary[]>("list_sessions", {}) : loadLocalSessions();
      if (!disposed && generation === sessionRefreshGenerationRef.current) setSessions(nextSessions);
    });
    const terminalPoll = shareRefresh(async () => {
      if (!isBackendAvailable()) return;
      const [eventsResult, oneKeysResult] = await Promise.allSettled([
        invokeBackend<SessionEvent[]>("tail_log", { sessionId: request.sessionId, limit: 600 }),
        invokeBackend<OneKeySummary[]>("list_one_keys", {}),
      ]);
      if (disposed) return;
      if (eventsResult.status === "fulfilled") {
        const nextSignature = detachedEventSignature(eventsResult.value);
        if (eventSignature !== nextSignature) {
          eventSignature = nextSignature;
          setEvents(eventsResult.value);
        }
      }
      if (oneKeysResult.status === "fulfilled") setOneKeys(oneKeysResult.value);
      if (eventsResult.status === "rejected") throw eventsResult.reason;
      if (oneKeysResult.status === "rejected") throw oneKeysResult.reason;
    });
    refreshNowRef.current = async () => {
      const results = await Promise.allSettled([sessionsPoll(), terminalPoll()]);
      for (const result of results) if (result.status === "rejected") throw result.reason;
    };
    // Background polls retain the last good snapshot. A manual refresh joins
    // any in-flight polls and reports their actual outcome to the user.
    const pollSessions = () => { void sessionsPoll().catch(() => {}); };
    const pollTerminal = () => { void terminalPoll().catch(() => {}); };
    pollSessions();
    pollTerminal();
    const sessionTimer = window.setInterval(pollSessions, 1_200);
    const terminalStateTimer = window.setInterval(pollTerminal, 2_000);
    return () => {
      disposed = true;
      refreshNowRef.current = null;
      window.clearInterval(sessionTimer);
      window.clearInterval(terminalStateTimer);
    };
  }, [request.sessionId]);

  useEffect(() => {
    if (!isBackendAvailable()) return;
    let disposed = false;
    const unlisten = new Set<() => void>();
    void listen<SessionEvent>("portmate-session-event-updated", (event) => {
      if (disposed || event.payload.sessionId !== request.sessionId) return;
      setEvents((current) => {
        const index = current.findIndex((candidate) => candidate.id === event.payload.id);
        if (index < 0) return current;
        const next = current.slice();
        next[index] = event.payload;
        return next;
      });
    }).then((nextUnlisten) => {
      if (disposed) nextUnlisten();
      else unlisten.add(nextUnlisten);
    }).catch(() => {});
    void listen<DeleteSessionProfileResponse | string>(SESSION_PROFILE_DELETED_EVENT, (event) => {
      const deletedProfileId = typeof event.payload === "string"
        ? event.payload
        : event.payload?.deletedProfileId;
      if (disposed || deletedProfileId !== request.sessionId) return;
      sessionRefreshGenerationRef.current += 1;
      setSessions((current) => current.filter((item) => item.profile.id !== request.sessionId));
      setError(t("session-profile-deleted"));
      void getCurrentWebviewWindow().close().catch(() => {});
    }).then((nextUnlisten) => {
      if (disposed) nextUnlisten();
      else unlisten.add(nextUnlisten);
    }).catch(() => {});
    void listen<SessionSummary>(SESSION_PROFILE_UPDATED_EVENT, (event) => {
      if (disposed || event.payload?.profile?.id !== request.sessionId) return;
      sessionRefreshGenerationRef.current += 1;
      setSessions((current) => upsertDetachedSessionSummary(current, event.payload));
    }).then((nextUnlisten) => {
      if (disposed) nextUnlisten();
      else unlisten.add(nextUnlisten);
    }).catch(() => {});
    return () => {
      disposed = true;
      for (const stopListening of unlisten) stopListening();
      unlisten.clear();
    };
  }, [request.sessionId]);

  useEffect(() => {
    const refreshLock = () => setScreenLock((current) => {
      const next = readScreenLockMarker(current?.lockedAt);
      return screenLockMarkersEqual(current, next) ? current : next;
    });
    const handleStorage = (event: StorageEvent) => {
      if (event.key === SCREEN_LOCK_STORAGE_KEY || event.key === null) refreshLock();
    };
    const timer = window.setInterval(refreshLock, 500);
    window.addEventListener("storage", handleStorage);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("storage", handleStorage);
    };
  }, []);

  useEffect(() => {
    const handleStorage = (event: StorageEvent) => {
      // Native events and backend refreshes own live session state. A peer's
      // cache may be stale or invalid and must not tear down this terminal.
      if (!isBackendAvailable() && (event.key === SESSION_SUMMARY_CACHE_STORAGE_KEY || event.key === null)) {
        sessionRefreshGenerationRef.current += 1;
        setSessions(loadLocalSessions());
      }
      if (["portmate.terminalPrefs", COMMAND_HISTORY_STORAGE_KEY, QUICK_COMMAND_STORAGE_KEY, null].includes(event.key)) {
        setTerminalInteractionPrefs(readTerminalInteractionPrefs(request.sessionId));
      }
    };
    window.addEventListener("storage", handleStorage);
    return () => window.removeEventListener("storage", handleStorage);
  }, []);

  useEffect(() => {
    const handleScreenLockShortcut = (event: KeyboardEvent) => {
      if (!isScreenLockShortcut(event)) return;
      event.preventDefault();
      event.stopPropagation();
      void sendOwnerCommand("lock-screen");
    };
    window.addEventListener("keydown", handleScreenLockShortcut, true);
    return () => window.removeEventListener("keydown", handleScreenLockShortcut, true);
  }, []);

  function enqueueTerminalInput(
    sessionId: string,
    text: string,
    origin: SyncInputOrigin,
    options?: TerminalInputSendOptions,
  ): void | Promise<void> {
    if (!isBackendAvailable()) return;
    return terminalInputRef.current!.dispatch(sessionId, text, origin, options);
  }

  function rememberDetachedCommand(sessionId: string, command: string, source: CommandSubmissionSource) {
    if (!terminalInteractionPrefs.historyEnabled) return;
    const policy = normalizeCommandHistoryPolicy(
      terminalInteractionPrefs.completionSettings.historyLimit,
      terminalInteractionPrefs.completionSettings.historyRetentionDays,
    );
    const localEntries = readCommandHistoryEntries(policy);
    const optimistic = recordCommandHistory(localEntries, command, policy, Date.now(), sessionId);
    persistDetachedCommandHistory(optimistic);
    setTerminalInteractionPrefs((current) => ({
      ...current,
      completionHistory: commandHistoryCommands(optimistic, request.sessionId),
    }));
    if (!isBackendAvailable()) return;
    commandHistoryOperationRef.current = commandHistoryOperationRef.current.then(async () => {
      try {
        const snapshot = await invokeBackend<CommandHistorySnapshot>("record_command_history", {
          command,
          sessionId,
          source,
        });
        const canonical = normalizeCommandHistory(
          { version: 3, entries: snapshot.entries },
          policy,
        );
        persistDetachedCommandHistory(canonical);
        setTerminalInteractionPrefs((current) => ({
          ...current,
          completionHistory: commandHistoryCommands(canonical, request.sessionId),
        }));
      } catch (error) {
        console.warn("PortMate detached command history persistence failed", error);
      }
    });
  }

  async function completeOneKeyPrompt(
    sessionId: string,
    oneKeyId: string,
    field: OneKeyPromptField,
    promptEventId: string,
  ) {
    if (!isBackendAvailable()) throw new Error("PortMate desktop backend is unavailable");
    const snapshot = terminalInputRef.current!.capture();
    if (!snapshot) return;
    await inputQueueRef.current.enqueue(async () => {
      if (!terminalInputRef.current!.isCurrent(snapshot)) return;
      try {
        await invokeBackend("send_one_key", {
          request: {
            id: oneKeyId,
            sessionId,
            field,
            source: "prompt-completion",
            promptEventId,
            expectedConnectedSince: snapshot.connectedSince,
          },
        });
      } catch (inputError) {
        if (terminalInputRef.current!.isCurrent(snapshot)) throw inputError;
      }
    });
  }

  async function sendOwnerCommand(action: DetachedPaneCommand["action"]) {
    const controlAction = action === "lock-screen" ? null : action;
    if (controlAction && ownerCommandBusyRef.current) return;
    if (controlAction) {
      ownerCommandBusyRef.current = controlAction;
      setOwnerCommandBusy(controlAction);
    }
    const command: DetachedPaneCommand = { ...request, title: viewTitle, keyMode, action, requestId: createDetachedCommandRequestId() };
    try {
      if (action === "reattach") {
        const result = await requestReattachResult(command);
        if (!result.ok) throw new Error(result.error);
      } else if (isBackendAvailable()) {
        await emitTo(request.ownerWindowId, DETACHED_PANE_EVENT, command);
      } else if (window.opener && !window.opener.closed) {
        window.opener.postMessage({ type: DETACHED_PANE_MESSAGE_TYPE, payload: command }, window.location.origin);
      } else {
        throw new Error(t("source-workspace-unavailable"));
      }
      setError("");
      if (action === "reattach") {
        if (isBackendAvailable()) await getCurrentWebviewWindow().close();
        else window.close();
      }
    } catch (commandError) {
      setError(formatDetachedError(commandError));
    } finally {
      if (controlAction && ownerCommandBusyRef.current === controlAction) {
        ownerCommandBusyRef.current = null;
        setOwnerCommandBusy(null);
      }
    }
  }

  useEffect(() => {
    const dismiss = () => setContextMenu(null);
    window.addEventListener(INTERACTION_LAYER_DISMISS_EVENT, dismiss);
    return () => window.removeEventListener(INTERACTION_LAYER_DISMISS_EVENT, dismiss);
  }, []);

  async function refreshInPlace() {
    if (refreshing || !refreshNowRef.current) return;
    setRefreshing(true);
    try {
      await refreshNowRef.current();
      setError("");
    } catch (refreshError) {
      setError(formatDetachedError(refreshError));
    } finally {
      setRefreshing(false);
    }
  }

  function openDetachedTerminalContextMenu(event: ReactMouseEvent) {
    const target = event.target instanceof Element ? event.target : null;
    const terminalHost = target?.closest<HTMLElement>(".terminal-host");
    if (!terminalHost || !session) return;
    event.preventDefault();
    event.stopPropagation();
    setContextMenu({
      x: event.clientX,
      y: event.clientY,
      alternate: terminalHost.dataset.terminalBuffer === "alternate",
      hasSelection: terminalHost.dataset.terminalHasSelection === "true",
    });
  }

  async function exportDetachedTerminalText(source: TerminalTextExportSource, destination: "default" | "choose" = "default") {
    if (!session || exportBusy) return;
    setExportBusy(true);
    try {
      const directory = normalizeTerminalExportDirectory(terminalInteractionPrefs.completionSettings.terminalTextExportDirectory);
      const desktop = isBackendAvailable();
      const destinationPath = desktop && destination === "choose"
        ? await chooseTerminalTextExportPath(directory, terminalTextExportFileName(session.profile.name, source))
        : null;
      if (desktop && destination === "choose" && destinationPath === null) return;
      if (!sessionsRef.current.some(item => item.profile.id === session.profile.id) || hasActiveModalLayer()) return;
      const payload = await requestTerminalTextExport({ sessionId: session.profile.id, viewId: request.viewId, source });
      if (!desktop) {
        downloadTerminalText(payload.text, session.profile.name, source);
        return;
      }
      await invokeBackend("export_terminal_text", { request: terminalTextExportRequest(payload, directory, destinationPath) });
    } catch (exportError) {
      setError(formatDetachedError(exportError));
    } finally {
      setExportBusy(false);
    }
  }

  function handleDetachedContextAction(action: TerminalContextAction) {
    setContextMenu(null);
    if (!session) return;
    const target = { sessionId: session.profile.id, viewId: request.viewId };
    switch (action) {
      case "copy":
        void import("./terminal-selection-event")
          .then(({ executeTerminalSelectionAction }) => executeTerminalSelectionAction({ ...target, action: "copy" }))
          .catch((actionError) => setError(formatDetachedError(actionError)));
        return;
      case "paste": {
        const lane = terminalInputRef.current;
        const snapshot = lane?.capture();
        if (!lane || !snapshot) return;
        void pasteClipboardIntoTerminalView(snapshot.sessionId, request.viewId, () => lane.isCurrent(snapshot))
          .catch((actionError) => setError(formatDetachedError(actionError)));
        return;
      }
      case "find":
        window.requestAnimationFrame(() => requestTerminalSearch());
        return;
      case "search-online":
        void import("./terminal-selection-event")
          .then(({ executeTerminalOnlineSearch }) => executeTerminalOnlineSearch({ ...target, fallback: session.lastLine }))
          .catch((actionError) => setError(formatDetachedError(actionError)));
        return;
      case "clear-scrollback":
      case "clear-screen":
      case "clear-all":
        void import("./terminal-buffer-event")
          .then(({ executeTerminalBufferAction }) => executeTerminalBufferAction({ ...target, action }))
          .catch((actionError) => setError(formatDetachedError(actionError)));
        return;
      case "select-all":
        void import("./terminal-selection-event")
          .then(({ executeTerminalSelectionAction }) => executeTerminalSelectionAction({ ...target, action: "select-all" }))
          .catch((actionError) => setError(formatDetachedError(actionError)));
        return;
      case "clear-selection":
        void import("./terminal-selection-event")
          .then(({ executeTerminalSelectionAction }) => executeTerminalSelectionAction({ ...target, action: "clear" }))
          .catch((actionError) => setError(formatDetachedError(actionError)));
        return;
      case "export-buffer":
        void exportDetachedTerminalText("buffer", "default");
        return;
      case "export-buffer-to":
        void exportDetachedTerminalText("buffer", "choose");
        return;
      case "export-selection":
        void exportDetachedTerminalText("selection", "default");
        return;
      default:
        return;
    }
  }

  return (
    <main
      className="detached-pane-root"
      data-window-id={request.windowId}
      data-pane-id={request.paneId}
      data-event-count={events.length}
      data-one-key-count={oneKeys.length}
      data-owner-command-busy={ownerCommandBusy ?? ""}
      onClick={() => setContextMenu(null)}
    >
      <header className={request.color ? "detached-pane-toolbar colored" : "detached-pane-toolbar"} style={request.color ? { borderTopColor: request.color } : undefined}>
        <span className="detached-brand"><PortMateIcon size={16} variant="mark" />PortMate</span>
        <strong>{viewTitle || session?.profile.name || t("session-unavailable")}</strong>
        {session && shellElevationRequested(session.profile) ? <ShellElevationIndicator profile={session.profile} /> : null}
        <span
          className={`session-status-dot status-${session?.runtime.status ?? "disconnected"}`}
          role="status"
          aria-label={runtimeHealth}
          title={runtimeHealth}
        />
        <span className="detached-endpoint">{session ? describeDetachedEndpoint(session) : request.sessionId}</span>
        <button type="button" title={t("refresh-session")} aria-label={t("refresh-session")} disabled={refreshing} onClick={() => void refreshInPlace()}>
          <RefreshCw size={14} className={refreshing ? "spin" : undefined} />
        </button>
        {connectionAction === "disconnect" ? (
          <button type="button" title={t("disconnect-session")} aria-label={t("disconnect-session")} disabled={ownerCommandBusy !== null} onClick={() => void sendOwnerCommand("disconnect")}>
            <Square size={13} />
          </button>
        ) : (
          <button type="button" title={t("connect-session")} aria-label={t("connect-session")} disabled={!session || ownerCommandBusy !== null} onClick={() => void sendOwnerCommand("connect")}>
            <Play size={14} />
          </button>
        )}
        <button type="button" title={t("return-to-workspace-2")} aria-label={t("return-to-workspace-2")} disabled={ownerCommandBusy !== null} onClick={() => void sendOwnerCommand("reattach")}>
          <PanelLeftOpen size={15} />
        </button>
      </header>
      <section className="detached-pane-terminal" onContextMenu={openDetachedTerminalContextMenu}>
        <TerminalCanvas viewId={request.viewId} active={session} events={events} focused oneKeys={oneKeys} oneKeyCompletionEnabled={terminalInteractionPrefs.oneKeyCompletionEnabled} completionSettings={terminalInteractionPrefs.completionSettings} completionHistory={terminalInteractionPrefs.completionHistory} completionQuickCommands={terminalInteractionPrefs.completionQuickCommands} mouseReporting={terminalInteractionPrefs.mouseReporting} copyOnSelect={terminalInteractionPrefs.copyOnSelect} keyMode={keyMode} onKeyModeChange={setKeyMode} onInput={enqueueTerminalInput} onCommandSubmit={rememberDetachedCommand} onOneKeyCompletion={completeOneKeyPrompt} />
      </section>
      <footer className={statusError ? "detached-pane-status error" : "detached-pane-status"}>
        <span title={statusText} aria-live="polite">{statusText}</span>
        <button
          type="button"
          data-key-mode={keyMode}
          title={keyModeShortcutHint}
          onClick={() => setKeyMode(toggleTerminalInsertNormalMode(keyMode))}
        >
          {terminalKeyModeLabel(keyMode)}
        </button>
      </footer>
      {contextMenu ? (
        <TerminalContextMenu
          state={contextMenu}
          exportBusy={exportBusy}
          showTriggers={false}
          onAction={handleDetachedContextAction}
        />
      ) : null}
      {screenLock ? <ChildWindowScreenLockOverlay marker={screenLock} ownerWindowId={request.ownerWindowId} /> : null}
    </main>
  );
}

async function requestReattachResult(command: DetachedPaneCommand): Promise<DetachedPaneResult> {
  if (isBackendAvailable()) return requestTauriReattachResult(command);
  return requestBrowserReattachResult(command);
}

async function requestTauriReattachResult(command: DetachedPaneCommand): Promise<DetachedPaneResult> {
  let resolveResult!: (result: DetachedPaneResult) => void;
  let rejectResult!: (error: Error) => void;
  const resultPromise = new Promise<DetachedPaneResult>((resolve, reject) => {
    resolveResult = resolve;
    rejectResult = reject;
  });
  const unlisten = await listen<unknown>(DETACHED_PANE_RESULT_EVENT, (event) => {
    const result = normalizeDetachedPaneResult(event.payload);
    if (result?.windowId === command.windowId && result.requestId === command.requestId) resolveResult(result);
  });
  const timeout = window.setTimeout(() => {
    rejectResult(new Error(t("the-main-window-did-not-acknowledge-the-return-within")));
  }, DETACHED_REATTACH_RESULT_TIMEOUT_MS);
  try {
    const [, result] = await Promise.all([
      emitTo(command.ownerWindowId, DETACHED_PANE_EVENT, command),
      resultPromise,
    ]);
    return result;
  } finally {
    window.clearTimeout(timeout);
    unlisten();
  }
}

async function requestBrowserReattachResult(command: DetachedPaneCommand): Promise<DetachedPaneResult> {
  const opener = window.opener;
  if (!opener || opener.closed) throw new Error(t("source-workspace-unavailable"));
  let timeout = 0;
  let handleMessage: ((event: MessageEvent) => void) | null = null;
  const resultPromise = new Promise<DetachedPaneResult>((resolve, reject) => {
    handleMessage = (event) => {
      if (event.origin !== window.location.origin || event.source !== opener) return;
      const message = normalizeDetachedPaneResultMessage(event.data);
      if (message?.payload.windowId === command.windowId && message.payload.requestId === command.requestId) resolve(message.payload);
    };
    window.addEventListener("message", handleMessage);
    timeout = window.setTimeout(() => {
      reject(new Error(t("the-main-window-did-not-acknowledge-the-return-within")));
    }, DETACHED_REATTACH_RESULT_TIMEOUT_MS);
  });
  try {
    opener.postMessage({
      type: DETACHED_PANE_MESSAGE_TYPE,
      payload: command,
    }, window.location.origin);
    return await resultPromise;
  } finally {
    window.clearTimeout(timeout);
    if (handleMessage) window.removeEventListener("message", handleMessage);
  }
}

function createDetachedCommandRequestId(): string {
  const uuid = globalThis.crypto?.randomUUID?.();
  return uuid
    ? `request-${uuid}`
    : `request-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 12)}`;
}

function shareRefresh(load: () => Promise<void>) {
  let pending: Promise<void> | null = null;
  return () => pending ??= load().finally(() => { pending = null; });
}

function loadLocalSessions(): SessionSummary[] {
  try {
    return readSessionSummaryCache(window.localStorage);
  } catch {
    return [];
  }
}

function readTerminalInteractionPrefs(sessionId: string): DetachedTerminalInteractionPrefs {
  const defaults: DetachedTerminalInteractionPrefs = {
    oneKeyCompletionEnabled: true,
    completionSettings: {},
    completionHistory: readCompletionHistory(sessionId),
    completionQuickCommands: readCompletionQuickCommands(),
    historyEnabled: true,
    mouseReporting: true,
    copyOnSelect: true,
  };
  try {
    const raw = window.localStorage.getItem("portmate.terminalPrefs");
    if (!raw) return defaults;
    const value = JSON.parse(raw) as Record<string, unknown>;
    return {
      oneKeyCompletionEnabled: typeof value.oneKeyCompletionEnabled === "boolean" ? value.oneKeyCompletionEnabled : true,
      completionSettings: value,
      completionHistory: readCompletionHistory(sessionId, value.historyLimit, value.historyRetentionDays),
      completionQuickCommands: readCompletionQuickCommands(),
      historyEnabled: typeof value.historyEnabled === "boolean" ? value.historyEnabled : true,
      mouseReporting: typeof value.mouseReporting === "boolean" ? value.mouseReporting : true,
      copyOnSelect: typeof value.mouseCopyOnSelect === "boolean" ? value.mouseCopyOnSelect : true,
    };
  } catch {
    return defaults;
  }
}

function readCompletionHistory(sessionId: string, limit?: unknown, retentionDays?: unknown): string[] {
  try {
    const value = JSON.parse(window.localStorage.getItem(COMMAND_HISTORY_STORAGE_KEY) ?? "null") as unknown;
    return commandHistoryCommands(normalizeCommandHistory(
      value,
      normalizeCommandHistoryPolicy(limit, retentionDays),
    ), sessionId);
  } catch {
    return [];
  }
}

function readCommandHistoryEntries(policy: ReturnType<typeof normalizeCommandHistoryPolicy>) {
  try {
    return normalizeCommandHistory(
      JSON.parse(window.localStorage.getItem(COMMAND_HISTORY_STORAGE_KEY) ?? "null") as unknown,
      policy,
    );
  } catch {
    return [];
  }
}

function persistDetachedCommandHistory(entries: Parameters<typeof commandHistorySnapshot>[0]) {
  try {
    window.localStorage.setItem(COMMAND_HISTORY_STORAGE_KEY, JSON.stringify(commandHistorySnapshot(entries)));
  } catch {
    // The detached terminal keeps its in-memory completion list when storage is unavailable.
  }
}

function readCompletionQuickCommands() {
  try {
    const value = JSON.parse(window.localStorage.getItem(QUICK_COMMAND_STORAGE_KEY) ?? "null") as unknown;
    return normalizeQuickCommandLibrary(value).items;
  } catch {
    return [];
  }
}

function readScreenLockMarker(fallbackLockedAt = Date.now()): ScreenLockMarker | null {
  try {
    const raw = window.localStorage.getItem(SCREEN_LOCK_STORAGE_KEY);
    return decodeStoredScreenLockMarker(raw, fallbackLockedAt)?.marker ?? null;
  } catch {
    return null;
  }
}

function screenLockMarkersEqual(left: ScreenLockMarker | null, right: ScreenLockMarker | null) {
  return left?.version === right?.version
    && left?.reason === right?.reason
    && left?.lockedAt === right?.lockedAt;
}

function describeDetachedEndpoint(session: SessionSummary) {
  const connection = session.profile.connection;
  if (connection.kind === "shell") return connection.program;
  if (connection.kind === "serial") return connection.port;
  if ("endpoint" in connection) {
    return `${connection.endpoint.host}:${connection.endpoint.port}`;
  }
  return `${connection.host}:${connection.port}`;
}

function detachedEventSignature(events: readonly SessionEvent[]) {
  const last = events.at(-1);
  return `${events.length}:${last?.id ?? ""}:${last?.ts ?? ""}`;
}

function formatDetachedError(error: unknown) {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}
