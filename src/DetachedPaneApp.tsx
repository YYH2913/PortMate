import { useEffect, useState } from "react";
import { emitTo } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { PanelLeftOpen, Play, RefreshCw, Square } from "lucide-react";
import { callBackend, invokeBackend, isBackendAvailable } from "./api";
import {
  DETACHED_PANE_EVENT,
  DETACHED_PANE_MESSAGE_TYPE,
} from "./detached-pane-state";
import type { DetachedPaneCommand, DetachedPaneRequest } from "./detached-pane-state";
import TerminalCanvas from "./TerminalCanvas";
import type { SessionEvent, SessionSummary } from "./types";

export default function DetachedPaneApp({ request }: { request: DetachedPaneRequest }) {
  const [sessions, setSessions] = useState<SessionSummary[]>(loadLocalSessions);
  const [events, setEvents] = useState<SessionEvent[]>([]);
  const [error, setError] = useState("");
  const session = sessions.find((item) => item.profile.id === request.sessionId);

  useEffect(() => {
    document.title = session ? `${request.title || session.profile.name} - PortMate` : "PortMate Detached Pane";
  }, [request.title, session?.profile.name]);

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
      const nextEvents = await callBackend("tail_log", { sessionId: request.sessionId, limit: 600 }, []);
      if (!disposed) setEvents(nextEvents);
    }
  }, [request.sessionId]);

  async function sendInput(sessionId: string, text: string) {
    if (!text || !isBackendAvailable()) return;
    try {
      await invokeBackend("send_text", { sessionId, text });
      setError("");
    } catch (inputError) {
      setError(formatDetachedError(inputError));
    }
  }

  async function sendMainCommand(action: DetachedPaneCommand["action"]) {
    const command: DetachedPaneCommand = { ...request, action };
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
        <TerminalCanvas active={session} events={events} focused onInput={(sessionId, text) => void sendInput(sessionId, text)} />
      </section>
      <footer className={error ? "detached-pane-status error" : "detached-pane-status"}>
        <span>{error || session?.runtime.status || "missing"}</span>
        <span>{request.paneId}</span>
      </footer>
    </main>
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
