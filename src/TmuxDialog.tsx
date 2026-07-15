import { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Check, Pencil, Play, Plus, Radio, RefreshCw, Trash2, X } from "lucide-react";
import { invokeBackend } from "./api";
import { groupTmuxPanes } from "./tmux-state";
import type { TmuxWindowGroup } from "./tmux-state";
import type {
  SessionEvent,
  SessionSummary,
  TmuxControlEvent,
  TmuxControlStatus,
  TmuxMutationAction,
  TmuxMutationRequest,
  TmuxState,
  TmuxWindowLayout,
} from "./types";

type TmuxEditor = {
  action: "rename-session" | "new-window" | "rename-window";
  target: string;
  value: string;
};

type TmuxDeleteConfirmation = {
  action: "kill-session" | "kill-window" | "kill-pane";
  target: string;
  label: string;
};

type TmuxPaneMutationAction =
  | "select-pane"
  | "split-pane-horizontal"
  | "split-pane-vertical"
  | "swap-pane-previous"
  | "swap-pane-next"
  | "resize-pane-left"
  | "resize-pane-right"
  | "resize-pane-up"
  | "resize-pane-down";

type TmuxPaneMoveOption = {
  key: string;
  label: string;
  action: "break-pane" | "move-pane-horizontal" | "move-pane-vertical";
  destination?: string;
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
  const [controlTarget, setControlTarget] = useState("");
  const [controlBusy, setControlBusy] = useState(false);
  const [editor, setEditor] = useState<TmuxEditor | null>(null);
  const [deleteConfirmation, setDeleteConfirmation] = useState<TmuxDeleteConfirmation | null>(null);
  const [feedback, setFeedback] = useState("");
  const [error, setError] = useState("");
  const mountedRef = useRef(true);
  const sessionIdRef = useRef(session.profile.id);
  const controlTargetRef = useRef("");
  const controlRuntimeIdRef = useRef("");
  const controlRequestedTargetRef = useRef("");
  const stoppedControlRuntimeIdsRef = useRef(new Set<string>());
  const controlOwnedRef = useRef(false);
  const controlRefreshInFlightRef = useRef(false);
  const controlRefreshPendingRef = useRef(false);
  sessionIdRef.current = session.profile.id;
  const windows = useMemo(() => groupTmuxPanes(state.panes, state.windows), [state.panes, state.windows]);
  const operationBusy = busy || controlBusy || Boolean(syncingTarget) || Boolean(mutatingTarget);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    const sessionId = session.profile.id;
    controlOwnedRef.current = false;
    controlTargetRef.current = "";
    controlRuntimeIdRef.current = "";
    controlRequestedTargetRef.current = "";
    stoppedControlRuntimeIdsRef.current.clear();
    controlRefreshPendingRef.current = false;
    setControlTarget("");
    setControlBusy(false);
    void refreshTmux();
    return () => {
      const owned = controlOwnedRef.current;
      controlOwnedRef.current = false;
      controlTargetRef.current = "";
      controlRuntimeIdRef.current = "";
      controlRequestedTargetRef.current = "";
      controlRefreshPendingRef.current = false;
      if (owned) {
        void invokeBackend<TmuxControlStatus>("stop_tmux_control", { sessionId }).catch(() => {});
      }
    };
  }, [session.profile.id]);

  useEffect(() => {
    const sessionId = session.profile.id;
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<TmuxControlEvent>("portmate-tmux-control-event", (event) => {
      if (disposed || event.payload.sessionId !== sessionId) return;
      const payload = event.payload;
      if (payload.kind === "started") {
        const requested = controlRequestedTargetRef.current;
        if (
          requested === payload.target
          || (!requested && (!controlTargetRef.current || controlTargetRef.current === payload.target))
        ) {
          controlRuntimeIdRef.current = payload.runtimeId;
          controlTargetRef.current = payload.target;
          setControlTarget(payload.target);
        }
        return;
      }
      if (payload.kind === "state-changed") {
        if (controlRuntimeIdRef.current === payload.runtimeId) {
          void refreshTmuxFromControl(sessionId);
        }
        return;
      }
      if (controlRequestedTargetRef.current === payload.target) {
        stoppedControlRuntimeIdsRef.current.add(payload.runtimeId);
      }
      if (controlRuntimeIdRef.current !== payload.runtimeId) return;
      controlOwnedRef.current = false;
      controlTargetRef.current = "";
      controlRuntimeIdRef.current = "";
      setControlTarget("");
      setControlBusy(false);
      if (payload.error) setError(payload.error);
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

  async function refreshTmuxFromControl(expectedSessionId: string) {
    controlRefreshPendingRef.current = true;
    if (controlRefreshInFlightRef.current) return;
    controlRefreshInFlightRef.current = true;
    try {
      while (
        controlRefreshPendingRef.current
        && mountedRef.current
        && sessionIdRef.current === expectedSessionId
      ) {
        controlRefreshPendingRef.current = false;
        try {
          const nextState = await invokeBackend<TmuxState>("list_tmux_state", { sessionId: expectedSessionId });
          if (mountedRef.current && sessionIdRef.current === expectedSessionId) setState(nextState);
        } catch (error) {
          if (mountedRef.current && sessionIdRef.current === expectedSessionId) {
            setError(formatTmuxError(error));
          }
        }
      }
    } finally {
      controlRefreshInFlightRef.current = false;
      if (controlRefreshPendingRef.current && mountedRef.current) {
        void refreshTmuxFromControl(sessionIdRef.current);
      }
    }
  }

  async function startControl(nextTarget: string) {
    const sessionId = session.profile.id;
    setControlBusy(true);
    setError("");
    setFeedback("");
    setEditor(null);
    setDeleteConfirmation(null);
    controlOwnedRef.current = true;
    controlRequestedTargetRef.current = nextTarget;
    try {
      const status = await invokeBackend<TmuxControlStatus>("start_tmux_control", {
        sessionId,
        target: nextTarget,
      });
      if (!mountedRef.current || sessionIdRef.current !== sessionId) {
        if (status.active) {
          void invokeBackend<TmuxControlStatus>("stop_tmux_control", { sessionId }).catch(() => {});
        }
        return;
      }
      controlRequestedTargetRef.current = "";
      if (status.runtimeId && stoppedControlRuntimeIdsRef.current.delete(status.runtimeId)) {
        controlOwnedRef.current = false;
        return;
      }
      controlOwnedRef.current = status.active;
      controlRuntimeIdRef.current = status.active ? status.runtimeId || "" : "";
      controlTargetRef.current = status.active ? status.target : "";
      setControlTarget(controlTargetRef.current);
      setFeedback(`${nextTarget} 已开启 control-mode 实时监听`);
    } catch (error) {
      controlOwnedRef.current = false;
      controlRequestedTargetRef.current = "";
      if (mountedRef.current && sessionIdRef.current === sessionId) {
        setError(formatTmuxError(error));
      }
    } finally {
      if (mountedRef.current && sessionIdRef.current === sessionId) setControlBusy(false);
    }
  }

  async function stopControl() {
    const sessionId = session.profile.id;
    const previousTarget = controlTargetRef.current;
    const wasOwned = controlOwnedRef.current;
    setControlBusy(true);
    setError("");
    setFeedback("");
    controlOwnedRef.current = false;
    controlRequestedTargetRef.current = "";
    try {
      await invokeBackend<TmuxControlStatus>("stop_tmux_control", { sessionId });
      if (!mountedRef.current || sessionIdRef.current !== sessionId) return;
      controlTargetRef.current = "";
      controlRuntimeIdRef.current = "";
      setControlTarget("");
      setFeedback(`${previousTarget} 已停止 control-mode 实时监听`);
    } catch (error) {
      controlOwnedRef.current = wasOwned;
      if (mountedRef.current && sessionIdRef.current === sessionId) {
        setError(formatTmuxError(error));
      }
    } finally {
      if (mountedRef.current && sessionIdRef.current === sessionId) setControlBusy(false);
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
    options: Pick<TmuxMutationRequest, "destination" | "layout" | "amount"> = {},
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
        ...options,
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
    const { action, target: deletionTarget, label } = deleteConfirmation;
    void mutate(
      action,
      deletionTarget,
      null,
      `${label} 已关闭`,
    );
  }

  function applyPaneMutation(action: TmuxPaneMutationAction, paneTarget: string, paneLabel: string) {
    setEditor(null);
    setDeleteConfirmation(null);
    const amount = action.startsWith("resize-pane-") ? 5 : null;
    void mutate(
      action,
      paneTarget,
      null,
      paneMutationFeedback(action, paneLabel),
      amount ? { amount } : {},
    );
  }

  function applyWindowLayout(windowTarget: string, layout: TmuxWindowLayout) {
    setEditor(null);
    setDeleteConfirmation(null);
    void mutate(
      "select-layout",
      windowTarget,
      null,
      `${windowTarget} 已应用 ${layout} 布局`,
      { layout },
    );
  }

  function applyPaneMove(option: TmuxPaneMoveOption, paneTarget: string, paneLabel: string) {
    setEditor(null);
    setDeleteConfirmation(null);
    const feedback = option.action === "break-pane"
      ? `${paneLabel} 已拆为新 window`
      : `${paneLabel} 已移到 ${option.destination}`;
    void mutate(
      option.action,
      paneTarget,
      null,
      feedback,
      option.destination ? { destination: option.destination } : {},
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
                    ? `关闭 session ${deleteConfirmation.label} 及其全部 window？`
                    : deleteConfirmation.action === "kill-window"
                      ? `关闭 window ${deleteConfirmation.label}？`
                      : `关闭 pane ${deleteConfirmation.label}？`}
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
                        <button
                          type="button"
                          className={controlTarget === item.name ? "tmux-control-active" : ""}
                          title={controlTarget === item.name ? `停止实时监听 session ${item.name}` : `实时监听 session ${item.name}`}
                          aria-label={controlTarget === item.name ? `停止实时监听 session ${item.name}` : `实时监听 session ${item.name}`}
                          aria-pressed={controlTarget === item.name}
                          disabled={operationBusy}
                          onClick={() => {
                            if (controlTarget === item.name) void stopControl();
                            else void startControl(item.name);
                          }}
                        ><Radio size={13} /></button>
                        <button type="button" title={`在 ${item.name} 新建 window`} aria-label={`在 ${item.name} 新建 window`} disabled={operationBusy} onClick={() => openEditor({ action: "new-window", target: item.name, value: "" })}><Plus size={13} /></button>
                        <button type="button" title={`重命名 session ${item.name}`} aria-label={`重命名 session ${item.name}`} disabled={operationBusy} onClick={() => openEditor({ action: "rename-session", target: item.name, value: item.name })}><Pencil size={13} /></button>
                        <button type="button" className="danger" title={`关闭 session ${item.name}`} aria-label={`关闭 session ${item.name}`} disabled={operationBusy} onClick={() => {
                          setError("");
                          setFeedback("");
                          setEditor(null);
                          setDeleteConfirmation({ action: "kill-session", target: item.name, label: item.name });
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
                      <select
                        className="tmux-layout-select"
                        aria-label={`${window.target} window 布局`}
                        title={`切换 ${window.target} 布局`}
                        defaultValue=""
                        disabled={operationBusy}
                        onChange={(event) => {
                          const layout = event.currentTarget.value as TmuxWindowLayout;
                          event.currentTarget.value = "";
                          if (layout) applyWindowLayout(window.target, layout);
                        }}
                      >
                        <option value="" disabled>布局</option>
                        <option value="even-horizontal">等宽左右</option>
                        <option value="even-vertical">等高上下</option>
                        <option value="main-horizontal">主窗格上方</option>
                        <option value="main-vertical">主窗格左侧</option>
                        <option value="tiled">平铺</option>
                      </select>
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
                          setDeleteConfirmation({ action: "kill-window", target: window.target, label: window.target });
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
                    {window.panes.map((pane) => {
                      const paneLabel = `${window.target}.${pane.paneIndex}`;
                      const moveOptions = buildPaneMoveOptions(windows, window.target, window.panes.length > 1);
                      return (
                        <div
                          key={pane.paneId || `${pane.session}-${pane.windowIndex}-${pane.paneIndex}`}
                          className={pane.active ? "active" : ""}
                          data-tmux-pane={pane.paneId}
                        >
                          <button
                            type="button"
                            className="tmux-pane-summary"
                            title={`激活 pane ${paneLabel}`}
                            aria-label={`激活 pane ${paneLabel}`}
                            disabled={operationBusy || !pane.paneId}
                            onClick={() => applyPaneMutation("select-pane", pane.paneId, paneLabel)}
                          >
                            <strong>{paneLabel}</strong>
                            <span>{pane.command || "shell"}</span>
                            <small>{pane.title || pane.paneId}</small>
                          </button>
                          <div className="tmux-pane-actions">
                          <select
                            aria-label={`${paneLabel} pane 分割`}
                            title={`分割 ${paneLabel}`}
                            defaultValue=""
                            disabled={operationBusy || !pane.paneId}
                            onChange={(event) => {
                              const action = event.currentTarget.value as TmuxPaneMutationAction;
                              event.currentTarget.value = "";
                              if (action) applyPaneMutation(action, pane.paneId, paneLabel);
                            }}
                          >
                            <option value="" disabled>分割</option>
                            <option value="split-pane-horizontal">左右</option>
                            <option value="split-pane-vertical">上下</option>
                          </select>
                          <select
                            aria-label={`${paneLabel} pane 交换`}
                            title={`交换 ${paneLabel}`}
                            defaultValue=""
                            disabled={operationBusy || !pane.paneId || window.panes.length < 2}
                            onChange={(event) => {
                              const action = event.currentTarget.value as TmuxPaneMutationAction;
                              event.currentTarget.value = "";
                              if (action) applyPaneMutation(action, pane.paneId, paneLabel);
                            }}
                          >
                            <option value="" disabled>交换</option>
                            <option value="swap-pane-previous">向前</option>
                            <option value="swap-pane-next">向后</option>
                          </select>
                          <select
                            aria-label={`${paneLabel} pane 调整尺寸`}
                            title={`调整 ${paneLabel} 尺寸`}
                            defaultValue=""
                            disabled={operationBusy || !pane.paneId}
                            onChange={(event) => {
                              const action = event.currentTarget.value as TmuxPaneMutationAction;
                              event.currentTarget.value = "";
                              if (action) applyPaneMutation(action, pane.paneId, paneLabel);
                            }}
                          >
                            <option value="" disabled>尺寸</option>
                            <option value="resize-pane-left">向左 5</option>
                            <option value="resize-pane-right">向右 5</option>
                            <option value="resize-pane-up">向上 5</option>
                            <option value="resize-pane-down">向下 5</option>
                          </select>
                          <select
                            className="tmux-pane-move-select"
                            aria-label={`${paneLabel} pane 移动`}
                            title={`移动 ${paneLabel}`}
                            defaultValue=""
                            disabled={operationBusy || !pane.paneId || !moveOptions.length}
                            onChange={(event) => {
                              const option = moveOptions.find((item) => item.key === event.currentTarget.value);
                              event.currentTarget.value = "";
                              if (option) applyPaneMove(option, pane.paneId, paneLabel);
                            }}
                          >
                            <option value="" disabled>移动</option>
                            {moveOptions.map((option) => (
                              <option value={option.key} key={option.key}>{option.label}</option>
                            ))}
                          </select>
                          <button
                            type="button"
                            className="danger"
                            title={`关闭 pane ${paneLabel}`}
                            aria-label={`关闭 pane ${paneLabel}`}
                            disabled={operationBusy || !pane.paneId}
                            onClick={() => {
                              setError("");
                              setFeedback("");
                              setEditor(null);
                              setDeleteConfirmation({
                                action: "kill-pane",
                                target: pane.paneId,
                                label: paneLabel,
                              });
                            }}
                          ><Trash2 size={13} /></button>
                          </div>
                        </div>
                      );
                    })}
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

function paneMutationFeedback(action: TmuxPaneMutationAction, paneLabel: string): string {
  switch (action) {
    case "select-pane": return `${paneLabel} 已激活`;
    case "split-pane-horizontal": return `${paneLabel} 已左右分割`;
    case "split-pane-vertical": return `${paneLabel} 已上下分割`;
    case "swap-pane-previous": return `${paneLabel} 已与前一 pane 交换`;
    case "swap-pane-next": return `${paneLabel} 已与后一 pane 交换`;
    case "resize-pane-left": return `${paneLabel} 已向左调整 5 cells`;
    case "resize-pane-right": return `${paneLabel} 已向右调整 5 cells`;
    case "resize-pane-up": return `${paneLabel} 已向上调整 5 cells`;
    case "resize-pane-down": return `${paneLabel} 已向下调整 5 cells`;
  }
}

function buildPaneMoveOptions(
  windows: readonly TmuxWindowGroup[],
  currentWindowTarget: string,
  allowBreak: boolean,
): TmuxPaneMoveOption[] {
  const options: TmuxPaneMoveOption[] = allowBreak
    ? [{ key: "break", label: "拆为新 window", action: "break-pane" }]
    : [];
  for (const destination of windows) {
    if (destination.target === currentWindowTarget) continue;
    options.push(
      {
        key: `horizontal:${destination.target}`,
        label: `${destination.target} · 左右`,
        action: "move-pane-horizontal",
        destination: destination.target,
      },
      {
        key: `vertical:${destination.target}`,
        label: `${destination.target} · 上下`,
        action: "move-pane-vertical",
        destination: destination.target,
      },
    );
  }
  return options;
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
