import { X } from "lucide-react";
import type { SessionEvent, SessionProfile, SessionSummary } from "./types";

export type SearchDialogState = { mode: "sessions" | "logs"; query: string };

export default function SearchDialog({
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
        .filter((event) => !query || `${event.text ?? ""} ${event.annotations.commandId ?? ""}`.toLowerCase().includes(query))
        .slice(-80)
        .reverse()
        .map((event) => {
          const commandId = event.annotations.commandId;
          const commandLabel = commandId ? `[命令 ${commandId.slice(0, 8)}] ` : "";
          return {
            id: event.sessionId,
            title: sessions.find((session) => session.profile.id === event.sessionId)?.profile.name ?? event.sessionId,
            detail: `${commandLabel}${event.text ?? ""}`,
          };
        });

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
