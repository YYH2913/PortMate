import { useEffect, useRef, useState } from "react";
import { emitTo } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow, WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { Lock, PanelLeftOpen, Play, RefreshCw, Square } from "lucide-react";
import { callBackend, invokeBackend, isBackendAvailable } from "./api";
import { COMMAND_HISTORY_STORAGE_KEY, commandHistoryCommands, normalizeCommandHistory, normalizeCommandHistoryPolicy } from "./command-history-state";
import {
  buildDetachedPanePath,
  DETACHED_PANE_EVENT,
  DETACHED_PANE_MESSAGE_TYPE,
} from "./detached-pane-state";
import type { DetachedPaneCommand, DetachedPaneRequest } from "./detached-pane-state";
import { decodeStoredScreenLockMarker, isScreenLockShortcut, SCREEN_LOCK_STORAGE_KEY } from "./screen-lock-state";
import type { ScreenLockMarker } from "./screen-lock-state";
import TerminalCanvas from "./TerminalCanvas";
import { normalizeQuickCommandLibrary, QUICK_COMMAND_STORAGE_KEY } from "./quick-command-state";
import type { OneKeyPromptField } from "./one-key-completion-state";
import type { OneKeySummary, SessionEvent, SessionSummary } from "./types";
import { terminalKeyModeLabel, toggleTerminalRemoteLocalMode } from "./terminal-key-mode";
import type { TerminalKeyMode } from "./terminal-key-mode";

export default function DetachedPaneApp({ request }: { request: DetachedPaneRequest }) {
  const [sessions, setSessions] = useState<SessionSummary[]>(loadLocalSessions);
  const [events, setEvents] = useState<SessionEvent[]>([]);
  const [oneKeys, setOneKeys] = useState<OneKeySummary[]>([]);
  const [terminalInteractionPrefs, setTerminalInteractionPrefs] = useState(readTerminalInteractionPrefs);
  const [keyMode, setKeyMode] = useState<TerminalKeyMode>(request.keyMode);
  const [error, setError] = useState("");
  const [screenLock, setScreenLock] = useState<ScreenLockMarker | null>(readScreenLockMarker);
  const session = sessions.find((item) => item.profile.id === request.sessionId);

  useEffect(() => {
    document.title = session ? `${request.title || session.profile.name} - PortMate` : "PortMate Detached Pane";
  }, [request.title, session?.profile.name]);

  useEffect(() => {
    window.history.replaceState(null, "", buildDetachedPanePath({ ...request, keyMode }));
  }, [keyMode, request]);

  useEffect(() => {
    let disposed = false;
    void refresh(true);
    const timer = window.setInterval(() => void refresh(false), 1_200);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };

    async function refresh(includeLog: boolean) {
      const nextSessions = await callBackend("list_sessions", {}, loadLocalSessions());
      if (disposed) return;
      setSessions(nextSessions);
      if (!includeLog) return;
      const [nextEvents, nextOneKeys] = await Promise.all([
        callBackend("tail_log", { sessionId: request.sessionId, limit: 600 }, []),
        callBackend("list_one_keys", {}, []),
      ]);
      if (!disposed) {
        setEvents(nextEvents);
        setOneKeys(nextOneKeys);
      }
    }
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
      void sendMainCommand("lock-screen");
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

  async function sendMainCommand(action: DetachedPaneCommand["action"]) {
    const command: DetachedPaneCommand = { ...request, keyMode, action };
    try {
      if (isBackendAvailable()) {
        await emitTo("main", DETACHED_PANE_EVENT, command);
      } else if (window.opener && !window.opener.closed) {
        window.opener.postMessage({ type: DETACHED_PANE_MESSAGE_TYPE, payload: command }, window.location.origin);
      } else {
        throw new Error("主窗口不可用");
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
    <main className="detached-pane-root" data-window-id={request.windowId} data-pane-id={request.paneId}>
      <header className={request.color ? "detached-pane-toolbar colored" : "detached-pane-toolbar"} style={request.color ? { borderTopColor: request.color } : undefined}>
        <span className="detached-brand">PortMate</span>
        <strong>{request.title || session?.profile.name || "会话不可用"}</strong>
        <span className={`tab-status ${session?.runtime.status ?? "disconnected"}`} />
        <span className="detached-endpoint">{session ? describeDetachedEndpoint(session) : request.sessionId}</span>
        <button type="button" title="刷新会话" aria-label="刷新会话" onClick={() => window.location.reload()}>
          <RefreshCw size={14} />
        </button>
        {session?.runtime.status === "connected" ? (
          <button type="button" title="断开会话" aria-label="断开会话" onClick={() => void sendMainCommand("disconnect")}>
            <Square size={13} />
          </button>
        ) : (
          <button type="button" title="连接会话" aria-label="连接会话" disabled={!session} onClick={() => void sendMainCommand("connect")}>
            <Play size={14} />
          </button>
        )}
        <button type="button" title="返回主窗口" aria-label="返回主窗口" onClick={() => void sendMainCommand("reattach")}>
          <PanelLeftOpen size={15} />
        </button>
      </header>
      <section className="detached-pane-terminal">
        <TerminalCanvas viewId={request.viewId} active={session} events={events} focused oneKeys={oneKeys} oneKeyCompletionEnabled={terminalInteractionPrefs.oneKeyCompletionEnabled} completionSettings={terminalInteractionPrefs.completionSettings} completionHistory={terminalInteractionPrefs.completionHistory} completionQuickCommands={terminalInteractionPrefs.completionQuickCommands} mouseReporting={terminalInteractionPrefs.mouseReporting} copyOnSelect={terminalInteractionPrefs.copyOnSelect} keyMode={keyMode} onKeyModeChange={setKeyMode} onInput={(sessionId, text) => void sendInput(sessionId, text)} onOneKeyCompletion={completeOneKeyPrompt} />
      </section>
      <footer className={error ? "detached-pane-status error" : "detached-pane-status"}>
        <span>{error || session?.runtime.status || "missing"}</span>
        <button
          type="button"
          data-key-mode={keyMode}
          title="切换远程/本地模式 (Ctrl+Enter)"
          onClick={() => setKeyMode(toggleTerminalRemoteLocalMode(keyMode))}
        >
          {terminalKeyModeLabel(keyMode)}
        </button>
      </footer>
      {screenLock ? <DetachedScreenLockOverlay marker={screenLock} /> : null}
    </main>
  );
}

function DetachedScreenLockOverlay({ marker }: { marker: ScreenLockMarker }) {
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

  async function focusMainWindow() {
    try {
      if (isBackendAvailable()) {
        const main = await WebviewWindow.getByLabel("main");
        await main?.setFocus();
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
        <p className="screen-lock-message">请在主窗口完成解锁</p>
        <button ref={buttonRef} className="screen-lock-primary" type="button" onClick={() => void focusMainWindow()}>
          <PanelLeftOpen size={15} />
          <span>切换到主窗口</span>
        </button>
      </section>
    </div>
  );
}

function loadLocalSessions(): SessionSummary[] {
  try {
    const raw = window.localStorage.getItem("portmate.sessions");
    return raw ? JSON.parse(raw) as SessionSummary[] : [];
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

function formatDetachedError(error: unknown) {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}
