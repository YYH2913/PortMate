import { useEffect, useRef, useState } from "react";
import { emitTo, listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow, WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { Lock, PanelLeftOpen, Play, RefreshCw, Square } from "lucide-react";
import { invokeBackend, isBackendAvailable } from "./api";
import { COMMAND_HISTORY_STORAGE_KEY, commandHistoryCommands, normalizeCommandHistory, normalizeCommandHistoryPolicy } from "./command-history-state";
import {
  buildDetachedPanePath,
  DETACHED_PANE_EVENT,
  DETACHED_PANE_MESSAGE_TYPE,
  SESSION_PROFILE_DELETED_EVENT,
  SESSION_PROFILE_UPDATED_EVENT,
  upsertDetachedSessionSummary,
} from "./detached-pane-state";
import type { DetachedPaneCommand, DetachedPaneRequest } from "./detached-pane-state";
import { decodeStoredScreenLockMarker, isScreenLockShortcut, SCREEN_LOCK_STORAGE_KEY } from "./screen-lock-state";
import type { ScreenLockMarker } from "./screen-lock-state";
import { sessionConnectionAction, sessionRuntimeHealthDescription } from "./session-runtime-state";
import { readSessionSummaryCache, SESSION_SUMMARY_CACHE_STORAGE_KEY } from "./session-summary-cache";
import TerminalCanvas from "./TerminalCanvas";
import { normalizeQuickCommandLibrary, QUICK_COMMAND_STORAGE_KEY } from "./quick-command-state";
import type { OneKeyPromptField } from "./one-key-completion-state";
import type { OneKeySummary, SessionEvent, SessionSummary } from "./types";
import { terminalKeyModeLabel, toggleTerminalInsertNormalMode } from "./terminal-key-mode";
import type { TerminalKeyMode } from "./terminal-key-mode";

export default function DetachedPaneApp({ request }: { request: DetachedPaneRequest }) {
  const [sessions, setSessions] = useState<SessionSummary[]>(loadLocalSessions);
  const [events, setEvents] = useState<SessionEvent[]>([]);
  const [oneKeys, setOneKeys] = useState<OneKeySummary[]>([]);
  const [terminalInteractionPrefs, setTerminalInteractionPrefs] = useState(readTerminalInteractionPrefs);
  const [keyMode, setKeyMode] = useState<TerminalKeyMode>(request.keyMode);
  const [error, setError] = useState("");
  const [screenLock, setScreenLock] = useState<ScreenLockMarker | null>(readScreenLockMarker);
  const sessionRefreshGenerationRef = useRef(0);
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
    void listen<string>(SESSION_PROFILE_DELETED_EVENT, (event) => {
      if (disposed || event.payload !== request.sessionId) return;
      sessionRefreshGenerationRef.current += 1;
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
        setTerminalInteractionPrefs(readTerminalInteractionPrefs());
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

  async function sendInput(sessionId: string, text: string) {
    if (!text || !isBackendAvailable()) return;
    try {
      await invokeBackend("send_text", { sessionId, text });
      setError("");
    } catch (inputError) {
      setError(formatDetachedError(inputError));
    }
  }

  async function completeOneKeyPrompt(
    sessionId: string,
    oneKeyId: string,
    field: OneKeyPromptField,
    promptEventId: string,
  ) {
    if (!isBackendAvailable()) throw new Error("PortMate desktop backend is unavailable");
    await invokeBackend("send_one_key", {
      request: {
        id: oneKeyId,
        sessionId,
        field,
        source: "prompt-completion",
        promptEventId,
      },
    });
  }

  async function sendOwnerCommand(action: DetachedPaneCommand["action"]) {
    const command: DetachedPaneCommand = { ...request, keyMode, action };
    try {
      if (isBackendAvailable()) {
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
    }
  }

  return (
    <main
      className="detached-pane-root"
      data-window-id={request.windowId}
      data-pane-id={request.paneId}
      data-event-count={events.length}
      data-one-key-count={oneKeys.length}
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
          <button type="button" title="断开会话" aria-label="断开会话" onClick={() => void sendOwnerCommand("disconnect")}>
            <Square size={13} />
          </button>
        ) : (
          <button type="button" title="连接会话" aria-label="连接会话" disabled={!session} onClick={() => void sendOwnerCommand("connect")}>
            <Play size={14} />
          </button>
        )}
        <button type="button" title="返回工作区" aria-label="返回工作区" onClick={() => void sendOwnerCommand("reattach")}>
          <PanelLeftOpen size={15} />
        </button>
      </header>
      <section className="detached-pane-terminal">
        <TerminalCanvas viewId={request.viewId} active={session} events={events} focused oneKeys={oneKeys} oneKeyCompletionEnabled={terminalInteractionPrefs.oneKeyCompletionEnabled} completionSettings={terminalInteractionPrefs.completionSettings} completionHistory={terminalInteractionPrefs.completionHistory} completionQuickCommands={terminalInteractionPrefs.completionQuickCommands} mouseReporting={terminalInteractionPrefs.mouseReporting} copyOnSelect={terminalInteractionPrefs.copyOnSelect} keyMode={keyMode} onKeyModeChange={setKeyMode} onInput={(sessionId, text) => void sendInput(sessionId, text)} onOneKeyCompletion={completeOneKeyPrompt} />
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
      {screenLock ? <DetachedScreenLockOverlay marker={screenLock} ownerWindowId={request.ownerWindowId} /> : null}
    </main>
  );
}

function DetachedScreenLockOverlay({ marker, ownerWindowId }: { marker: ScreenLockMarker; ownerWindowId: string }) {
  const overlayRef = useRef<HTMLDivElement | null>(null);
  const buttonRef = useRef<HTMLButtonElement | null>(null);

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
    buttonRef.current?.focus({ preventScroll: true });
    return () => {
      siblings.forEach((element, index) => {
        element.inert = previousInert[index];
      });
      if (previousFocus?.isConnected) previousFocus.focus({ preventScroll: true });
    };
  }, []);

  async function focusOwnerWorkspace() {
    try {
      if (isBackendAvailable()) {
        const owner = await WebviewWindow.getByLabel(ownerWindowId);
        await owner?.setFocus();
        return;
      }
      window.opener?.focus();
    } catch {
      window.opener?.focus();
    }
  }

  const reason = marker.reason === "idle" ? "空闲超时" : marker.reason === "startup" ? "启动保护" : "手动锁定";
  return (
    <div
      ref={overlayRef}
      className="screen-lock-overlay"
      role="dialog"
      aria-modal="true"
      aria-labelledby="detached-screen-lock-title"
      onKeyDown={(event) => {
        if (event.key === "Escape" || event.key === "Tab") {
          event.preventDefault();
          event.stopPropagation();
          buttonRef.current?.focus();
        }
      }}
    >
      <section className="screen-lock-panel">
        <div className="screen-lock-brand">
          <span className="screen-lock-icon"><Lock size={20} /></span>
          <span>PortMate</span>
        </div>
        <div className="screen-lock-heading">
          <h1 id="detached-screen-lock-title">屏幕已锁定</h1>
          <span>{reason} · {new Date(marker.lockedAt).toLocaleTimeString()}</span>
        </div>
        <div className="screen-lock-rule" />
        <p className="screen-lock-message">请在来源工作区完成解锁</p>
        <button ref={buttonRef} className="screen-lock-primary" type="button" onClick={() => void focusOwnerWorkspace()}>
          <PanelLeftOpen size={15} />
          <span>切换到来源工作区</span>
        </button>
      </section>
    </div>
  );
}

function loadLocalSessions(): SessionSummary[] {
  try {
    return readSessionSummaryCache(window.localStorage);
  } catch {
    return [];
  }
}

function readTerminalInteractionPrefs() {
  const defaults = {
    oneKeyCompletionEnabled: true,
    completionSettings: {},
    completionHistory: readCompletionHistory(),
    completionQuickCommands: readCompletionQuickCommands(),
    mouseReporting: true,
    copyOnSelect: true,
  };
  try {
    const raw = window.localStorage.getItem("portmate.terminalPrefs");
    if (!raw) return defaults;
    const value = JSON.parse(raw) as {
      oneKeyCompletionEnabled?: unknown;
      historyLimit?: unknown;
      historyRetentionDays?: unknown;
      mouseReporting?: unknown;
      mouseCopyOnSelect?: unknown;
    };
    return {
      oneKeyCompletionEnabled: typeof value.oneKeyCompletionEnabled === "boolean" ? value.oneKeyCompletionEnabled : true,
      completionSettings: value,
      completionHistory: readCompletionHistory(value.historyLimit, value.historyRetentionDays),
      completionQuickCommands: readCompletionQuickCommands(),
      mouseReporting: typeof value.mouseReporting === "boolean" ? value.mouseReporting : true,
      copyOnSelect: typeof value.mouseCopyOnSelect === "boolean" ? value.mouseCopyOnSelect : true,
    };
  } catch {
    return defaults;
  }
}

function readCompletionHistory(limit?: unknown, retentionDays?: unknown): string[] {
  try {
    const value = JSON.parse(window.localStorage.getItem(COMMAND_HISTORY_STORAGE_KEY) ?? "null") as unknown;
    return commandHistoryCommands(normalizeCommandHistory(
      value,
      normalizeCommandHistoryPolicy(limit, retentionDays),
    ));
  } catch {
    return [];
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
