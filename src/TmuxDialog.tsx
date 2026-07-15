import { useEffect, useMemo, useState } from "react";
import { Check, Pencil, Play, Plus, RefreshCw, Trash2, X } from "lucide-react";
import { invokeBackend } from "./api";
import { groupTmuxPanes } from "./tmux-state";
import type { SessionEvent, SessionSummary, TmuxMutationAction, TmuxMutationRequest, TmuxState } from "./types";

type TmuxEditor = {
  action: "rename-session" | "new-window" | "rename-window";
  target: string;
  value: string;
};

type TmuxDeleteConfirmation = {
  action: "kill-session" | "kill-window";
  target: string;
};

export default function TmuxDialog({
  session,
  onClose,
  onDone,
}: {
  session: SessionSummary;
  onClose: () => void;
  onDone: (message: string) => void;
}) {
  const [state, setState] = useState<TmuxState>({ sessions: [], windows: [], panes: [] });
  const [target, setTarget] = useState("portmate");
  const [busy, setBusy] = useState(false);
  const [syncingTarget, setSyncingTarget] = useState("");
  const [mutatingTarget, setMutatingTarget] = useState("");
  const [editor, setEditor] = useState<TmuxEditor | null>(null);
  const [deleteConfirmation, setDeleteConfirmation] = useState<TmuxDeleteConfirmation | null>(null);
  const [feedback, setFeedback] = useState("");
  const [error, setError] = useState("");
  const windows = useMemo(() => groupTmuxPanes(state.panes, state.windows), [state.panes, state.windows]);
  const operationBusy = busy || Boolean(syncingTarget) || Boolean(mutatingTarget);

  useEffect(() => {
    void refreshTmux();
  }, [session.profile.id]);

  async function refreshTmux() {
    setBusy(true);
    setError("");
    setFeedback("");
    setEditor(null);
    setDeleteConfirmation(null);
    try {
      const nextState = await invokeBackend<TmuxState>("list_tmux_state", { sessionId: session.profile.id });
      setState(nextState);
      setTarget((current) => current || nextState.sessions[0]?.name || "portmate");
    } catch (error) {
      setState({ sessions: [], windows: [], panes: [] });
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
    setEditor(null);
    setDeleteConfirmation(null);
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

  async function mutate(
    action: TmuxMutationAction,
    mutationTarget: string,
    name: string | null,
    successMessage: string,
  ) {
    setMutatingTarget(mutationTarget);
    setError("");
    setFeedback("");
    try {
      const request: TmuxMutationRequest = {
        sessionId: session.profile.id,
        action,
        target: mutationTarget,
        name,
      };
      const nextState = await invokeBackend<TmuxState>("mutate_tmux", { request });
      setState(nextState);
      if (action === "rename-session" && name && target === mutationTarget) setTarget(name);
      if (action === "kill-session" && target === mutationTarget) {
        setTarget(nextState.sessions[0]?.name || "portmate");
      }
      setEditor(null);
      setDeleteConfirmation(null);
      setFeedback(successMessage);
    } catch (error) {
      setError(formatTmuxError(error));
    } finally {
      setMutatingTarget("");
    }
  }

  function submitEditor() {
    if (!editor) return;
    const name = editor.value.trim();
    if (editor.action === "rename-session") {
      if (!name) return;
      void mutate("rename-session", editor.target, name, `${editor.target} 已重命名为 ${name}`);
      return;
    }
    if (editor.action === "rename-window") {
      if (!name) return;
      void mutate("rename-window", editor.target, name, `${editor.target} 已重命名为 ${name}`);
      return;
    }
    void mutate("new-window", editor.target, name || null, `${editor.target} 已新建 window`);
  }

  function openEditor(next: TmuxEditor) {
    setError("");
    setFeedback("");
    setDeleteConfirmation(null);
    setEditor(next);
  }

  function confirmDelete() {
    if (!deleteConfirmation) return;
    const { action, target: deletionTarget } = deleteConfirmation;
    void mutate(
      action,
      deletionTarget,
      null,
      `${deletionTarget} 已关闭`,
    );
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
            <input aria-label="Tmux session target" maxLength={256} value={target} onChange={(event) => setTarget(event.target.value)} placeholder="session name" />
            <button type="button" onClick={() => void attach()} disabled={operationBusy || !target.trim()}><Play size={14} />附着/新建</button>
            <button type="button" onClick={() => void refreshTmux()} disabled={operationBusy}><RefreshCw size={14} />刷新</button>
          </div>
          <div className="tmux-feedback" aria-live="polite">
            {error ? <div className="utility-error">{error}</div> : null}
            {feedback ? <div className="utility-success" role="status">{feedback}</div> : null}
            {deleteConfirmation ? (
              <div className="tmux-delete-confirmation" role="alert">
                <span>
                  {deleteConfirmation.action === "kill-session"
                    ? `关闭 session ${deleteConfirmation.target} 及其全部 window？`
                    : `关闭 window ${deleteConfirmation.target}？`}
                </span>
                <button type="button" onClick={() => setDeleteConfirmation(null)} disabled={Boolean(mutatingTarget)}>取消</button>
                <button type="button" className="danger" onClick={confirmDelete} disabled={Boolean(mutatingTarget)}>
                  {mutatingTarget ? "关闭中" : "确认关闭"}
                </button>
              </div>
            ) : null}
          </div>
          <section className="tmux-section">
            <h2>会话</h2>
            <div className="tmux-list">
              {state.sessions.map((item) => (
                <div className="tmux-session-row" data-tmux-session={item.name} key={item.name}>
                  {editor?.target === item.name && editor.action !== "rename-window" ? (
                    <TmuxInlineEditor
                      editor={editor}
                      busy={operationBusy}
                      onChange={(value) => setEditor((current) => current ? { ...current, value } : null)}
                      onSubmit={submitEditor}
                      onCancel={() => setEditor(null)}
                    />
                  ) : (
                    <>
                      <button className="tmux-session-main" type="button" disabled={operationBusy} onClick={() => {
                        setTarget(item.name);
                        void attach(item.name);
                      }}>
                        <strong>{item.name}</strong>
                        <span>{item.windows} windows · {item.attached} attached</span>
                        <small>{item.created ? new Date(item.created).toLocaleString() : "created time unavailable"}</small>
                      </button>
                      <div className="tmux-row-actions">
                        <button type="button" title={`在 ${item.name} 新建 window`} aria-label={`在 ${item.name} 新建 window`} disabled={operationBusy} onClick={() => openEditor({ action: "new-window", target: item.name, value: "" })}><Plus size={13} /></button>
                        <button type="button" title={`重命名 session ${item.name}`} aria-label={`重命名 session ${item.name}`} disabled={operationBusy} onClick={() => openEditor({ action: "rename-session", target: item.name, value: item.name })}><Pencil size={13} /></button>
                        <button type="button" className="danger" title={`关闭 session ${item.name}`} aria-label={`关闭 session ${item.name}`} disabled={operationBusy} onClick={() => {
                          setError("");
                          setFeedback("");
                          setEditor(null);
                          setDeleteConfirmation({ action: "kill-session", target: item.name });
                        }}><Trash2 size={13} /></button>
                      </div>
                    </>
                  )}
                </div>
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
                      <small>{window.name || window.windowId || "unnamed"} · {window.panes.length} panes{window.active ? " · active" : ""}</small>
                    </span>
                    <div className="tmux-window-controls">
                      <label className="tmux-sync-toggle">
                        <input
                          type="checkbox"
                          role="switch"
                          aria-label={`${window.target} pane 同步输入`}
                          checked={window.synchronized}
                          disabled={operationBusy}
                          onChange={(event) => void setPaneSync(window.target, event.currentTarget.checked)}
                        />
                        <span>{syncingTarget === window.target ? "应用中" : "同步输入"}</span>
                      </label>
                      <div className="tmux-row-actions">
                        <button type="button" title={`重命名 window ${window.target}`} aria-label={`重命名 window ${window.target}`} disabled={operationBusy} onClick={() => openEditor({ action: "rename-window", target: window.target, value: window.name })}><Pencil size={13} /></button>
                        <button type="button" className="danger" title={`关闭 window ${window.target}`} aria-label={`关闭 window ${window.target}`} disabled={operationBusy} onClick={() => {
                          setError("");
                          setFeedback("");
                          setEditor(null);
                          setDeleteConfirmation({ action: "kill-window", target: window.target });
                        }}><Trash2 size={13} /></button>
                      </div>
                    </div>
                  </header>
                  {editor?.action === "rename-window" && editor.target === window.target ? (
                    <TmuxInlineEditor
                      editor={editor}
                      busy={operationBusy}
                      onChange={(value) => setEditor((current) => current ? { ...current, value } : null)}
                      onSubmit={submitEditor}
                      onCancel={() => setEditor(null)}
                    />
                  ) : null}
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

function TmuxInlineEditor({
  editor,
  busy,
  onChange,
  onSubmit,
  onCancel,
}: {
  editor: TmuxEditor;
  busy: boolean;
  onChange: (value: string) => void;
  onSubmit: () => void;
  onCancel: () => void;
}) {
  const rename = editor.action !== "new-window";
  return (
    <form className="tmux-inline-editor" onSubmit={(event) => {
      event.preventDefault();
      onSubmit();
    }}>
      <input
        autoFocus
        aria-label={editor.action === "new-window" ? `新 window 名称 ${editor.target}` : `新名称 ${editor.target}`}
        maxLength={128}
        placeholder={editor.action === "new-window" ? "window name（可选）" : "new name"}
        value={editor.value}
        disabled={busy}
        onChange={(event) => onChange(event.currentTarget.value)}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            onCancel();
          }
        }}
      />
      <button type="submit" title="保存" aria-label="保存" disabled={busy || (rename && !editor.value.trim())}><Check size={14} /></button>
      <button type="button" title="取消" aria-label="取消" disabled={busy} onClick={onCancel}><X size={14} /></button>
    </form>
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
