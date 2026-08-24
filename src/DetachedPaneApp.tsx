import { useEffect, useRef, useState } from "react";
import { emitTo, listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { PanelLeftOpen, Play, RefreshCw, Square } from "lucide-react";
import { invokeBackend, isBackendAvailable } from "./api";
import { AsyncOperationQueue } from "./async-operation-queue";
import ChildWindowScreenLockOverlay from "./ChildWindowScreenLockOverlay";
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
import { TerminalInputPumpRegistry } from "./terminal-input-pump";
import { normalizeQuickCommandLibrary, QUICK_COMMAND_STORAGE_KEY } from "./quick-command-state";
import type { OneKeyPromptField } from "./one-key-completion-state";
import type { CommandHistorySnapshot, DeleteSessionProfileResponse, OneKeySummary, SessionEvent, SessionSummary } from "./types";
import { terminalKeyModeLabel, toggleTerminalInsertNormalMode } from "./terminal-key-mode";
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
  const [sessions, setSessions] = useState<SessionSummary[]>(loadLocalSessions);
  const [events, setEvents] = useState<SessionEvent[]>([]);
  const [oneKeys, setOneKeys] = useState<OneKeySummary[]>([]);
  const [terminalInteractionPrefs, setTerminalInteractionPrefs] = useState(() => readTerminalInteractionPrefs(request.sessionId));
  const [keyMode, setKeyMode] = useState<TerminalKeyMode>(request.keyMode);
  const [error, setError] = useState("");
  const errorRef = useRef(error);
  const [screenLock, setScreenLock] = useState<ScreenLockMarker | null>(readScreenLockMarker);
  const [ownerCommandBusy, setOwnerCommandBusy] = useState<DetachedOwnerControlAction | null>(null);
  const sessionRefreshGenerationRef = useRef(0);
  const ownerCommandBusyRef = useRef<DetachedOwnerControlAction | null>(null);
  const commandHistoryOperationRef = useRef<Promise<void>>(Promise.resolve());
  const inputQueueRef = useRef(new AsyncOperationQueue());
  const directInputPumpRef = useRef<TerminalInputPumpRegistry | null>(null);
  const inputEpochRef = useRef(0);
  const profileDeletedRef = useRef(false);
  errorRef.current = error;
  if (!directInputPumpRef.current) {
    directInputPumpRef.current = new TerminalInputPumpRegistry((sessionId, text, origin) => {
      const inputEpoch = captureTerminalInputEpoch();
      return inputEpoch === null
        ? Promise.resolve()
        : sendInput(sessionId, text, origin, inputEpoch);
    });
  }
  const session = sessions.find((item) => item.profile.id === request.sessionId);
  const connectionAction = session ? sessionConnectionAction(session.runtime.status) : "connect";
  const runtimeHealth = session ? sessionRuntimeHealthDescription(session.runtime) : "会话不可用";
  const statusText = error || runtimeHealth;
  const statusError = Boolean(error) || session?.runtime.status === "blocked" || session?.runtime.status === "error";

  useEffect(() => {
    document.title = session ? `${request.title || session.profile.name} - PortMate` : "PortMate Detached Pane";
  }, [request.title, session?.profile.name]);

  useEffect(() => {
    window.history.replaceState(null, "", buildDetachedPanePath({ ...request, keyMode }));
  }, [keyMode, request]);

  useEffect(() => {
    let disposed = false;
    let sessionsRefreshing = false;
    let terminalStateRefreshing = false;
    let eventSignature = "";
    void refreshSessions();
    void refreshTerminalState();
    const sessionTimer = window.setInterval(() => void refreshSessions(), 1_200);
    const terminalStateTimer = window.setInterval(() => void refreshTerminalState(), 2_000);
    return () => {
      disposed = true;
      window.clearInterval(sessionTimer);
      window.clearInterval(terminalStateTimer);
    };

    async function refreshSessions() {
      if (sessionsRefreshing) return;
      sessionsRefreshing = true;
      const generation = sessionRefreshGenerationRef.current;
      try {
        const nextSessions = isBackendAvailable()
          ? await invokeBackend<SessionSummary[]>("list_sessions", {})
          : loadLocalSessions();
        if (!disposed && generation === sessionRefreshGenerationRef.current) {
          if (nextSessions.some((item) => item.profile.id === request.sessionId)) restoreTerminalInput();
          setSessions(nextSessions);
        }
      } catch {
        // Keep the last confirmed session state during transient backend failures.
      } finally {
        sessionsRefreshing = false;
      }
    }

    async function refreshTerminalState() {
      if (terminalStateRefreshing) return;
      terminalStateRefreshing = true;
      try {
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
      } finally {
        terminalStateRefreshing = false;
      }
    }
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
      invalidateTerminalInput();
      setSessions((current) => current.filter((item) => item.profile.id !== request.sessionId));
      setError("会话 Profile 已删除");
      void getCurrentWebviewWindow().close().catch(() => {});
    }).then((nextUnlisten) => {
      if (disposed) nextUnlisten();
      else unlisten.add(nextUnlisten);
    }).catch(() => {});
    void listen<SessionSummary>(SESSION_PROFILE_UPDATED_EVENT, (event) => {
      if (disposed || event.payload?.profile?.id !== request.sessionId) return;
      sessionRefreshGenerationRef.current += 1;
      restoreTerminalInput();
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
      if (event.key === SESSION_SUMMARY_CACHE_STORAGE_KEY || event.key === null) {
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

  function captureTerminalInputEpoch(): number | null {
    return profileDeletedRef.current ? null : inputEpochRef.current;
  }

  function terminalInputIsCurrent(inputEpoch: number) {
    return !profileDeletedRef.current && inputEpochRef.current === inputEpoch;
  }

  function invalidateTerminalInput() {
    profileDeletedRef.current = true;
    inputEpochRef.current += 1;
    directInputPumpRef.current?.reset(request.sessionId);
  }

  function restoreTerminalInput() {
    profileDeletedRef.current = false;
  }

  function enqueueTerminalInput(
    sessionId: string,
    text: string,
    origin: SyncInputOrigin,
  ): Promise<void> {
    if (captureTerminalInputEpoch() === null) return Promise.resolve();
    if (origin === "interactive") {
      directInputPumpRef.current?.enqueueFast(sessionId, text, origin);
      return Promise.resolve();
    }
    return directInputPumpRef.current?.enqueue(sessionId, text, origin) ?? Promise.resolve();
  }

  function rememberDetachedCommand(sessionId: string, command: string) {
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
          limit: policy.limit,
          retentionDays: policy.retentionDays,
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

  async function sendInput(
    sessionId: string,
    text: string,
    origin: SyncInputOrigin,
    inputEpoch: number,
  ) {
    if (!text || !isBackendAvailable() || !terminalInputIsCurrent(inputEpoch)) return;
    try {
      await invokeBackend("send_text", {
        sessionId,
        text,
        interactive: origin === "interactive",
        queued: true,
      });
      if (terminalInputIsCurrent(inputEpoch) && errorRef.current) setError("");
    } catch (inputError) {
      if (terminalInputIsCurrent(inputEpoch)) setError(formatDetachedError(inputError));
    }
  }

  async function completeOneKeyPrompt(
    sessionId: string,
    oneKeyId: string,
    field: OneKeyPromptField,
    promptEventId: string,
  ) {
    if (!isBackendAvailable()) throw new Error("PortMate desktop backend is unavailable");
    const inputEpoch = captureTerminalInputEpoch();
    if (inputEpoch === null) return;
    await inputQueueRef.current.enqueue(async () => {
      if (!terminalInputIsCurrent(inputEpoch)) return;
      try {
        await invokeBackend("send_one_key", {
          request: {
            id: oneKeyId,
            sessionId,
            field,
            source: "prompt-completion",
            promptEventId,
          },
        });
      } catch (inputError) {
        if (terminalInputIsCurrent(inputEpoch)) throw inputError;
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
    const command: DetachedPaneCommand = { ...request, keyMode, action, requestId: createDetachedCommandRequestId() };
    try {
      if (action === "reattach") {
        const result = await requestReattachResult(command);
        if (!result.ok) throw new Error(result.error);
      } else if (isBackendAvailable()) {
        await emitTo(request.ownerWindowId, DETACHED_PANE_EVENT, command);
      } else if (window.opener && !window.opener.closed) {
        window.opener.postMessage({ type: DETACHED_PANE_MESSAGE_TYPE, payload: command }, window.location.origin);
      } else {
        throw new Error("来源工作区不可用");
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

  return (
    <main
      className="detached-pane-root"
      data-window-id={request.windowId}
      data-pane-id={request.paneId}
      data-event-count={events.length}
      data-one-key-count={oneKeys.length}
      data-owner-command-busy={ownerCommandBusy ?? ""}
    >
      <header className={request.color ? "detached-pane-toolbar colored" : "detached-pane-toolbar"} style={request.color ? { borderTopColor: request.color } : undefined}>
        <span className="detached-brand">PortMate</span>
        <strong>{request.title || session?.profile.name || "会话不可用"}</strong>
        <span
          className={`session-status-dot status-${session?.runtime.status ?? "disconnected"}`}
          role="status"
          aria-label={runtimeHealth}
          title={runtimeHealth}
        />
        <span className="detached-endpoint">{session ? describeDetachedEndpoint(session) : request.sessionId}</span>
        <button type="button" title="刷新会话" aria-label="刷新会话" onClick={() => window.location.reload()}>
          <RefreshCw size={14} />
        </button>
        {connectionAction === "disconnect" ? (
          <button type="button" title="断开会话" aria-label="断开会话" disabled={ownerCommandBusy !== null} onClick={() => void sendOwnerCommand("disconnect")}>
            <Square size={13} />
          </button>
        ) : (
          <button type="button" title="连接会话" aria-label="连接会话" disabled={!session || ownerCommandBusy !== null} onClick={() => void sendOwnerCommand("connect")}>
            <Play size={14} />
          </button>
        )}
        <button type="button" title="返回工作区" aria-label="返回工作区" disabled={ownerCommandBusy !== null} onClick={() => void sendOwnerCommand("reattach")}>
          <PanelLeftOpen size={15} />
        </button>
      </header>
      <section className="detached-pane-terminal">
        <TerminalCanvas viewId={request.viewId} active={session} events={events} focused oneKeys={oneKeys} oneKeyCompletionEnabled={terminalInteractionPrefs.oneKeyCompletionEnabled} completionSettings={terminalInteractionPrefs.completionSettings} completionHistory={terminalInteractionPrefs.completionHistory} completionQuickCommands={terminalInteractionPrefs.completionQuickCommands} mouseReporting={terminalInteractionPrefs.mouseReporting} copyOnSelect={terminalInteractionPrefs.copyOnSelect} keyMode={keyMode} onKeyModeChange={setKeyMode} onInput={enqueueTerminalInput} onCommandSubmit={rememberDetachedCommand} onOneKeyCompletion={completeOneKeyPrompt} />
      </section>
      <footer className={statusError ? "detached-pane-status error" : "detached-pane-status"}>
        <span title={statusText} aria-live="polite">{statusText}</span>
        <button
          type="button"
          data-key-mode={keyMode}
          title="切换 Insert/Normal 模式 (Esc / i)"
          onClick={() => setKeyMode(toggleTerminalInsertNormalMode(keyMode))}
        >
          {terminalKeyModeLabel(keyMode)}
        </button>
      </footer>
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
    rejectResult(new Error("主窗口未在 5 秒内确认返回结果。"));
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
  if (!opener || opener.closed) throw new Error("来源工作区不可用");
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
      reject(new Error("主窗口未在 5 秒内确认返回结果。"));
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
