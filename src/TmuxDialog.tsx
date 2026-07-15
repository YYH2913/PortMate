import { useEffect, useMemo, useState } from "react";
import { Play, RefreshCw, X } from "lucide-react";
import { invokeBackend } from "./api";
import { groupTmuxPanes } from "./tmux-state";
import type { SessionEvent, SessionSummary, TmuxState } from "./types";

export default function TmuxDialog({
  session,
  onClose,
  onDone,
}: {
  session: SessionSummary;
  onClose: () => void;
  onDone: (message: string) => void;
}) {
  const [state, setState] = useState<TmuxState>({ sessions: [], panes: [] });
  const [target, setTarget] = useState("portmate");
  const [busy, setBusy] = useState(false);
  const [syncingTarget, setSyncingTarget] = useState("");
  const [feedback, setFeedback] = useState("");
  const [error, setError] = useState("");
  const windows = useMemo(() => groupTmuxPanes(state.panes), [state.panes]);

  useEffect(() => {
    void refreshTmux();
  }, [session.profile.id]);

  async function refreshTmux() {
    setBusy(true);
    setError("");
    setFeedback("");
    try {
      const nextState = await invokeBackend<TmuxState>("list_tmux_state", { sessionId: session.profile.id });
      setState(nextState);
      setTarget((current) => current || nextState.sessions[0]?.name || "portmate");
    } catch (error) {
      setState({ sessions: [], panes: [] });
      setError(formatTmuxError(error));
    } finally {
      setBusy(false);
    }
  }

  async function attach(nextTarget = target) {
    const cleanTarget = nextTarget.trim();
    if (!cleanTarget) return;
    setBusy(true);
    setError("");
    setFeedback("");
    try {
      await invokeBackend<SessionEvent>("attach_tmux", { sessionId: session.profile.id, target: cleanTarget });
      onDone(`已发送 tmux attach/new-session：${cleanTarget}`);
    } catch (error) {
      setError(formatTmuxError(error));
    } finally {
      setBusy(false);
    }
  }

  async function setPaneSync(nextTarget: string, enabled: boolean) {
    setSyncingTarget(nextTarget);
    setError("");
    setFeedback("");
    try {
      const nextState = await invokeBackend<TmuxState>("set_tmux_pane_sync", {
        sessionId: session.profile.id,
        target: nextTarget,
        enabled,
      });
      setState(nextState);
      setFeedback(`${nextTarget} 已${enabled ? "开启" : "关闭"} pane 同步输入`);
    } catch (error) {
      setError(formatTmuxError(error));
    } finally {
      setSyncingTarget("");
    }
  }

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section className="wind-dialog tmux-dialog">
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>Tmux</strong>
          <button type="button" onClick={onClose}><X size={20} /></button>
        </header>
        <div className="tmux-content">
          <div className="tmux-toolbar">
            <input value={target} onChange={(event) => setTarget(event.target.value)} placeholder="session name" />
            <button type="button" onClick={() => void attach()} disabled={busy || !target.trim()}><Play size={14} />附着/新建</button>
            <button type="button" onClick={() => void refreshTmux()} disabled={busy}><RefreshCw size={14} />刷新</button>
          </div>
          <div className="tmux-feedback" aria-live="polite">
            {error ? <div className="utility-error">{error}</div> : null}
            {feedback ? <div className="utility-success" role="status">{feedback}</div> : null}
          </div>
          <section className="tmux-section">
            <h2>会话</h2>
            <div className="tmux-list">
              {state.sessions.map((item) => (
                <button type="button" key={item.name} onClick={() => {
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
            <h2>窗口与窗格</h2>
            <div className="tmux-window-list">
              {windows.map((window) => (
                <article className="tmux-window" data-tmux-target={window.target} key={window.target}>
                  <header>
                    <span>
                      <strong>{window.target}</strong>
                      <small>{window.panes.length} panes</small>
                    </span>
                    <label className="tmux-sync-toggle">
                      <input
                        type="checkbox"
                        role="switch"
                        aria-label={`${window.target} pane 同步输入`}
                        checked={window.synchronized}
                        disabled={busy || Boolean(syncingTarget)}
                        onChange={(event) => void setPaneSync(window.target, event.currentTarget.checked)}
                      />
                      <span>{syncingTarget === window.target ? "应用中" : "同步输入"}</span>
                    </label>
                  </header>
                  <div className="tmux-window-panes">
                    {window.panes.map((pane) => (
                      <div key={pane.paneId || `${pane.session}-${pane.windowIndex}-${pane.paneIndex}`} className={pane.active ? "active" : ""}>
                        <strong>{window.target}.{pane.paneIndex}</strong>
                        <span>{pane.command || "shell"}</span>
                        <small>{pane.title || pane.paneId}</small>
                      </div>
                    ))}
                  </div>
                </article>
              ))}
              {!windows.length ? <div className="empty-pane top">没有可显示的 pane</div> : null}
            </div>
          </section>
        </div>
      </section>
    </div>
  );
}

function formatTmuxError(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}
