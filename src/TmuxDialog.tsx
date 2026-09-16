import { t, useLocale, localizeDiagnostic } from "./i18n";
import { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Check, Pencil, Play, Plus, Radio, RefreshCw, Trash2, X } from "lucide-react";
import { invokeBackend } from "./api";
import { KeyedRequestGate } from "./keyed-request-gate";
import { groupTmuxPanes, rememberStoppedTmuxControlRuntimeId } from "./tmux-state";
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
  useLocale();
  const [state, setState] = useState<TmuxState>({ sessions: [], windows: [], panes: [] });
  const [target, setTarget] = useState("portmate");
  const [busy, setBusy] = useState(false);
  const [syncingTarget, setSyncingTarget] = useState("");
  const [mutatingTarget, setMutatingTarget] = useState("");
  const [controlRuntimes, setControlRuntimes] = useState<Map<string, string>>(() => new Map());
  const [controlBusyTargets, setControlBusyTargets] = useState<Set<string>>(() => new Set());
  const [editor, setEditor] = useState<TmuxEditor | null>(null);
  const [deleteConfirmation, setDeleteConfirmation] = useState<TmuxDeleteConfirmation | null>(null);
  const [feedback, setFeedback] = useState("");
  const [error, setError] = useState("");
  const mountedRef = useRef(true);
  const sessionIdRef = useRef(session.profile.id);
  const controlRuntimesRef = useRef(new Map<string, string>());
  const controlRequestedTargetsRef = useRef(new Set<string>());
  const stoppedControlRuntimeIdsRef = useRef(new Set<string>());
  const controlOwnedTargetsRef = useRef(new Set<string>());
  const controlRefreshInFlightRef = useRef(false);
  const controlRefreshPendingRef = useRef(false);
  const stateRequestGateRef = useRef(new KeyedRequestGate<"state">());
  const operationGateRef = useRef(new KeyedRequestGate<"operation">());
  const controlOperationGateRef = useRef(new KeyedRequestGate<string>());
  sessionIdRef.current = session.profile.id;
  const windows = useMemo(() => groupTmuxPanes(state.panes, state.windows), [state.panes, state.windows]);
  const operationBusy = busy || Boolean(syncingTarget) || Boolean(mutatingTarget);
  const controlOperationBusy = Boolean(controlBusyTargets.size);

  function publishControlRuntimes() {
    setControlRuntimes(new Map(controlRuntimesRef.current));
  }

  function setControlRuntime(target: string, runtimeId: string | null, expectedRuntimeId?: string) {
    if (expectedRuntimeId && controlRuntimesRef.current.get(target) !== expectedRuntimeId) return;
    if (runtimeId) controlRuntimesRef.current.set(target, runtimeId);
    else controlRuntimesRef.current.delete(target);
    publishControlRuntimes();
  }

  function setControlTargetBusy(target: string, value: boolean) {
    setControlBusyTargets((current) => {
      const next = new Set(current);
      if (value) next.add(target);
      else next.delete(target);
      return next;
    });
  }

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    const sessionId = session.profile.id;
    stateRequestGateRef.current.invalidate("state");
    controlOwnedTargetsRef.current.clear();
    controlRuntimesRef.current.clear();
    controlRequestedTargetsRef.current.clear();
    stoppedControlRuntimeIdsRef.current.clear();
    operationGateRef.current.invalidateAll();
    controlOperationGateRef.current.invalidateAll();
    controlRefreshPendingRef.current = false;
    setBusy(false);
    setSyncingTarget("");
    setMutatingTarget("");
    setControlRuntimes(new Map());
    setControlBusyTargets(new Set());
    void refreshTmux();
    return () => {
      stateRequestGateRef.current.invalidate("state");
      operationGateRef.current.invalidateAll();
      controlOperationGateRef.current.invalidateAll();
      const ownedRuntimes = [...controlOwnedTargetsRef.current].flatMap((target) => {
        const runtimeId = controlRuntimesRef.current.get(target);
        return runtimeId ? [{ target, runtimeId }] : [];
      });
      controlOwnedTargetsRef.current.clear();
      controlRuntimesRef.current.clear();
      controlRequestedTargetsRef.current.clear();
      controlRefreshPendingRef.current = false;
      for (const { target, runtimeId } of ownedRuntimes) {
        void invokeBackend<TmuxControlStatus>("stop_tmux_control", { sessionId, target, runtimeId }).catch(() => {});
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
        if (controlRequestedTargetsRef.current.has(payload.target)) {
          setControlRuntime(payload.target, payload.runtimeId);
        }
        return;
      }
      if (payload.kind === "state-changed") {
        if (controlRuntimesRef.current.get(payload.target) === payload.runtimeId) {
          void refreshTmuxFromControl(sessionId);
        }
        return;
      }
      if (controlRequestedTargetsRef.current.has(payload.target)) {
        rememberStoppedTmuxControlRuntimeId(stoppedControlRuntimeIdsRef.current, payload.runtimeId);
      }
      if (controlRuntimesRef.current.get(payload.target) !== payload.runtimeId) return;
      controlOwnedTargetsRef.current.delete(payload.target);
      setControlRuntime(payload.target, null, payload.runtimeId);
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
    const sessionId = session.profile.id;
    const operationGate = operationGateRef.current;
    const operationToken = operationGate.begin("operation");
    if (operationToken === null) return;
    const gate = stateRequestGateRef.current;
    const token = gate.replace("state");
    setBusy(true);
    setError("");
    setFeedback("");
    setEditor(null);
    setDeleteConfirmation(null);
    try {
      const nextState = await invokeBackend<TmuxState>("list_tmux_state", { sessionId });
      if (!isCurrentStateRequest(sessionId, token)) return;
      setState(nextState);
      setTarget((current) => current || nextState.sessions[0]?.name || "portmate");
    } catch (error) {
      if (!isCurrentStateRequest(sessionId, token)) return;
      setState({ sessions: [], windows: [], panes: [] });
      setError(formatTmuxError(error));
    } finally {
      gate.finish("state", token);
      if (operationGate.finish("operation", operationToken)
        && mountedRef.current && sessionIdRef.current === sessionId) setBusy(false);
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
        const gate = stateRequestGateRef.current;
        const token = gate.replace("state");
        try {
          const nextState = await invokeBackend<TmuxState>("list_tmux_state", { sessionId: expectedSessionId });
          if (isCurrentStateRequest(expectedSessionId, token)) setState(nextState);
        } catch (error) {
          if (isCurrentStateRequest(expectedSessionId, token)) {
            setError(formatTmuxError(error));
          }
        } finally {
          gate.finish("state", token);
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
    const gate = controlOperationGateRef.current;
    const token = gate.begin(nextTarget);
    if (token === null) return;
    setControlTargetBusy(nextTarget, true);
    setError("");
    setFeedback("");
    setEditor((current) => current?.target === nextTarget ? null : current);
    setDeleteConfirmation((current) => current?.target === nextTarget ? null : current);
    controlOwnedTargetsRef.current.add(nextTarget);
    controlRequestedTargetsRef.current.add(nextTarget);
    try {
      const status = await invokeBackend<TmuxControlStatus>("start_tmux_control", {
        sessionId,
        target: nextTarget,
      });
      if (!isCurrentControlOperation(sessionId, nextTarget, token)) {
        if (status.active && status.runtimeId) {
          void invokeBackend<TmuxControlStatus>("stop_tmux_control", {
            sessionId,
            target: nextTarget,
            runtimeId: status.runtimeId,
          }).catch(() => {});
        }
        return;
      }
      controlRequestedTargetsRef.current.delete(nextTarget);
      if (status.runtimeId && stoppedControlRuntimeIdsRef.current.delete(status.runtimeId)) {
        controlOwnedTargetsRef.current.delete(nextTarget);
        return;
      }
      const runtimeId = status.runtimeId || controlRuntimesRef.current.get(nextTarget) || "";
      if (!status.active || !runtimeId) {
        controlOwnedTargetsRef.current.delete(nextTarget);
        setControlRuntime(nextTarget, null);
        throw new Error(t("tmux-control-mode-did-not-return-a-valid-runtime"));
      }
      setControlRuntime(nextTarget, runtimeId);
      setFeedback(t("control-mode-live-monitoring-enabled", [nextTarget]));
    } catch (error) {
      if (!isCurrentControlOperation(sessionId, nextTarget, token)) return;
      controlOwnedTargetsRef.current.delete(nextTarget);
      controlRequestedTargetsRef.current.delete(nextTarget);
      setError(formatTmuxError(error));
    } finally {
      if (gate.finish(nextTarget, token)
        && mountedRef.current && sessionIdRef.current === sessionId) setControlTargetBusy(nextTarget, false);
    }
  }

  async function stopControl(target: string) {
    const sessionId = session.profile.id;
    const gate = controlOperationGateRef.current;
    const token = gate.begin(target);
    if (token === null) return;
    const previousRuntimeId = controlRuntimesRef.current.get(target) || "";
    const wasOwned = controlOwnedTargetsRef.current.has(target);
    setControlTargetBusy(target, true);
    setError("");
    setFeedback("");
    controlOwnedTargetsRef.current.delete(target);
    controlRequestedTargetsRef.current.delete(target);
    try {
      const status = await invokeBackend<TmuxControlStatus>("stop_tmux_control", {
        sessionId,
        target,
        runtimeId: previousRuntimeId || null,
      });
      if (!isCurrentControlOperation(sessionId, target, token)) return;
      if (status.active) {
        if (status.runtimeId) setControlRuntime(target, status.runtimeId);
        setError(t("tmux-control-mode-runtime-changed-retry-stopping-it"));
        return;
      }
      setControlRuntime(target, null, previousRuntimeId);
      setFeedback(t("control-mode-live-monitoring-stopped", [target]));
    } catch (error) {
      if (!isCurrentControlOperation(sessionId, target, token)) return;
      if (wasOwned) controlOwnedTargetsRef.current.add(target);
      setError(formatTmuxError(error));
    } finally {
      if (gate.finish(target, token)
        && mountedRef.current && sessionIdRef.current === sessionId) setControlTargetBusy(target, false);
    }
  }

  async function attach(nextTarget = target) {
    const cleanTarget = nextTarget.trim();
    if (!cleanTarget) return;
    const sessionId = session.profile.id;
    const operationGate = operationGateRef.current;
    const operationToken = operationGate.begin("operation");
    if (operationToken === null) return;
    setBusy(true);
    setError("");
    setFeedback("");
    try {
      await invokeBackend<SessionEvent>("attach_tmux", { sessionId, target: cleanTarget });
      if (!isCurrentOperation(sessionId, operationToken)) return;
      onDone(t("sent-tmux-attach-new-session", [cleanTarget]));
    } catch (error) {
      if (isCurrentOperation(sessionId, operationToken)) setError(formatTmuxError(error));
    } finally {
      if (operationGate.finish("operation", operationToken)
        && mountedRef.current && sessionIdRef.current === sessionId) setBusy(false);
    }
  }

  async function setPaneSync(nextTarget: string, enabled: boolean) {
    const sessionId = session.profile.id;
    const operationGate = operationGateRef.current;
    const operationToken = operationGate.begin("operation");
    if (operationToken === null) return;
    const gate = stateRequestGateRef.current;
    const token = gate.replace("state");
    setSyncingTarget(nextTarget);
    setError("");
    setFeedback("");
    setEditor(null);
    setDeleteConfirmation(null);
    try {
      const nextState = await invokeBackend<TmuxState>("set_tmux_pane_sync", {
        sessionId,
        target: nextTarget,
        enabled,
      });
      if (isCurrentStateRequest(sessionId, token)) setState(nextState);
      if (isCurrentOperation(sessionId, operationToken)) {
        setFeedback(t("pane-synchronized-input", [nextTarget, enabled ? t("enabled") : t("close")]));
      }
    } catch (error) {
      if (isCurrentOperation(sessionId, operationToken)) setError(formatTmuxError(error));
    } finally {
      gate.finish("state", token);
      if (operationGate.finish("operation", operationToken)
        && mountedRef.current && sessionIdRef.current === sessionId) {
        setSyncingTarget((current) => current === nextTarget ? "" : current);
      }
    }
  }

  async function mutate(
    action: TmuxMutationAction,
    mutationTarget: string,
    name: string | null,
    successMessage: string,
    options: Pick<TmuxMutationRequest, "destination" | "layout" | "amount"> = {},
  ) {
    const sessionId = session.profile.id;
    const operationGate = operationGateRef.current;
    const operationToken = operationGate.begin("operation");
    if (operationToken === null) return;
    const gate = stateRequestGateRef.current;
    const token = gate.replace("state");
    setMutatingTarget(mutationTarget);
    setError("");
    setFeedback("");
    try {
      const request: TmuxMutationRequest = {
        sessionId,
        action,
        target: mutationTarget,
        name,
        ...options,
      };
      const nextState = await invokeBackend<TmuxState>("mutate_tmux", { request });
      if (isCurrentStateRequest(sessionId, token)) setState(nextState);
      if (!isCurrentOperation(sessionId, operationToken)) return;
      if (action === "rename-session" && name && target === mutationTarget) setTarget(name);
      if (action === "kill-session" && target === mutationTarget) {
        setTarget(nextState.sessions[0]?.name || "portmate");
      }
      setEditor(null);
      setDeleteConfirmation(null);
      setFeedback(successMessage);
    } catch (error) {
      if (isCurrentOperation(sessionId, operationToken)) setError(formatTmuxError(error));
    } finally {
      gate.finish("state", token);
      if (operationGate.finish("operation", operationToken)
        && mountedRef.current && sessionIdRef.current === sessionId) {
        setMutatingTarget((current) => current === mutationTarget ? "" : current);
      }
    }
  }

  function isCurrentStateRequest(sessionId: string, token: number) {
    return mountedRef.current
      && sessionIdRef.current === sessionId
      && stateRequestGateRef.current.isCurrent("state", token);
  }

  function isCurrentOperation(sessionId: string, token: number) {
    return mountedRef.current
      && sessionIdRef.current === sessionId
      && operationGateRef.current.isCurrent("operation", token);
  }

  function isCurrentControlOperation(sessionId: string, target: string, token: number) {
    return mountedRef.current
      && sessionIdRef.current === sessionId
      && controlOperationGateRef.current.isCurrent(target, token);
  }

  function submitEditor() {
    if (!editor) return;
    const name = editor.value.trim();
    if (editor.action === "rename-session") {
      if (!name) return;
      void mutate("rename-session", editor.target, name, t("renamed-to", [editor.target, name]));
      return;
    }
    if (editor.action === "rename-window") {
      if (!name) return;
      void mutate("rename-window", editor.target, name, t("renamed-to", [editor.target, name]));
      return;
    }
    void mutate("new-window", editor.target, name || null, t("new-window-created-in", [editor.target]));
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
      t("closed", [label]),
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
      t("applied-layout", [windowTarget, layout]),
      { layout },
    );
  }

  function applyPaneMove(option: TmuxPaneMoveOption, paneTarget: string, paneLabel: string) {
    setEditor(null);
    setDeleteConfirmation(null);
    const feedback = option.action === "break-pane"
      ? t("split-into-a-new-window", [paneLabel])
      : t("moved-to", [paneLabel, option.destination]);
    void mutate(
      option.action,
      paneTarget,
      null,
      feedback,
      option.destination ? { destination: option.destination } : {},
    );
  }

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => {
      if (!controlOperationBusy && event.target === event.currentTarget) onClose();
    }}>
      <section className="wind-dialog tmux-dialog">
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>Tmux</strong>
          <button
            type="button"
            title={controlOperationBusy ? t("wait-for-the-control-mode-operation-to-finish") : t("close-tmux")}
            aria-label={t("close-tmux")}
            disabled={controlOperationBusy}
            onClick={onClose}
          ><X size={20} /></button>
        </header>
        <div className="tmux-content">
          <div className="tmux-toolbar">
            <input aria-label={t("ui-tmux-session-target")} maxLength={256} value={target} onChange={(event) => setTarget(event.target.value)} placeholder={t("ui-session-name")} />
            <button type="button" onClick={() => void attach()} disabled={operationBusy || controlBusyTargets.has(target.trim()) || !target.trim()}><Play size={14} />{t("attach-create")}</button>
            <button type="button" onClick={() => void refreshTmux()} disabled={operationBusy}><RefreshCw size={14} />{t("refresh")}</button>
          </div>
          <div className="tmux-feedback" aria-live="polite">
            {error ? <div className="utility-error">{localizeDiagnostic(error)}</div> : null}
            {feedback ? <div className="utility-success" role="status">{feedback}</div> : null}
            {deleteConfirmation ? (
              <div className="tmux-delete-confirmation" role="alert">
                <span>
                  {deleteConfirmation.action === "kill-session"
                    ? t("close-session-and-all-its-windows", [deleteConfirmation.label])
                    : deleteConfirmation.action === "kill-window"
                      ? t("close-window-2", [deleteConfirmation.label])
                      : t("close-pane", [deleteConfirmation.label])}
                </span>
                <button type="button" onClick={() => setDeleteConfirmation(null)} disabled={Boolean(mutatingTarget)}>{t("cancel")}</button>
                <button type="button" className="danger" onClick={confirmDelete} disabled={Boolean(mutatingTarget)}>
                  {mutatingTarget ? t("closing") : t("confirm-close")}
                </button>
              </div>
            ) : null}
          </div>
          <section className="tmux-section">
            <h2>{t("session")}</h2>
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
                      <button className="tmux-session-main" type="button" disabled={operationBusy || controlBusyTargets.has(item.name)} onClick={() => {
                        setTarget(item.name);
                        void attach(item.name);
                      }}>
                        <strong>{item.name}</strong>
                        <span>{item.windows}{" "}{t("ui-windows")}{" "}{item.attached}{" "}{t("ui-attached")}</span>
                        <small>{item.created ? new Date(item.created).toLocaleString() : t("ui-created-time-unavailable")}</small>
                      </button>
                      <div className="tmux-row-actions">
                        <button
                          type="button"
                          className={controlRuntimes.has(item.name) ? "tmux-control-active" : ""}
                          title={controlRuntimes.has(item.name) ? t("stop-live-monitoring-session", [item.name]) : t("monitor-session-live", [item.name])}
                          aria-label={controlRuntimes.has(item.name) ? t("stop-live-monitoring-session", [item.name]) : t("monitor-session-live", [item.name])}
                          aria-pressed={controlRuntimes.has(item.name)}
                          disabled={operationBusy || controlBusyTargets.has(item.name)}
                          onClick={() => {
                            if (controlRuntimes.has(item.name)) void stopControl(item.name);
                            else void startControl(item.name);
                          }}
                        ><Radio size={13} /></button>
                        <button type="button" title={t("create-window-in", [item.name])} aria-label={t("create-window-in", [item.name])} disabled={operationBusy || controlBusyTargets.has(item.name)} onClick={() => openEditor({ action: "new-window", target: item.name, value: "" })}><Plus size={13} /></button>
                        <button type="button" title={t("rename-session-2", [item.name])} aria-label={t("rename-session-2", [item.name])} disabled={operationBusy || controlBusyTargets.has(item.name)} onClick={() => openEditor({ action: "rename-session", target: item.name, value: item.name })}><Pencil size={13} /></button>
                        <button type="button" className="danger" title={t("close-session", [item.name])} aria-label={t("close-session", [item.name])} disabled={operationBusy || controlBusyTargets.has(item.name)} onClick={() => {
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
              {!state.sessions.length ? <div className="empty-pane top">{t("no-tmux-sessions-detected")}</div> : null}
            </div>
          </section>
          <section className="tmux-section">
            <h2>{t("windows-and-panes")}</h2>
            <div className="tmux-window-list">
              {windows.map((window) => (
                <article className="tmux-window" data-tmux-target={window.target} key={window.target}>
                  <header>
                    <span>
                      <strong>{window.target}</strong>
                      <small>{window.name || window.windowId || t("ui-unnamed")} · {window.panes.length}{" "}{t("ui-panes")}{window.active ? t("ui-active-suffix") : ""}</small>
                    </span>
                    <div className="tmux-window-controls">
                      <select
                        className="tmux-layout-select"
                        aria-label={t("window-layout", [window.target])}
                        title={t("change-layout", [window.target])}
                        defaultValue=""
                        disabled={operationBusy}
                        onChange={(event) => {
                          const layout = event.currentTarget.value as TmuxWindowLayout;
                          event.currentTarget.value = "";
                          if (layout) applyWindowLayout(window.target, layout);
                        }}
                      >
                        <option value="" disabled>{t("layout")}</option>
                        <option value="even-horizontal">{t("equal-width-columns")}</option>
                        <option value="even-vertical">{t("equal-height-rows")}</option>
                        <option value="main-horizontal">{t("main-pane-on-top")}</option>
                        <option value="main-vertical">{t("main-pane-on-left")}</option>
                        <option value="tiled">{t("tiled")}</option>
                      </select>
                      <label className="tmux-sync-toggle">
                        <input
                          type="checkbox"
                          role="switch"
                          aria-label={t("pane-synchronized-input-2", [window.target])}
                          checked={window.synchronized}
                          disabled={operationBusy}
                          onChange={(event) => void setPaneSync(window.target, event.currentTarget.checked)}
                        />
                        <span>{syncingTarget === window.target ? t("applying") : t("synchronized-input")}</span>
                      </label>
                      <div className="tmux-row-actions">
                        <button type="button" title={t("rename-window", [window.target])} aria-label={t("rename-window", [window.target])} disabled={operationBusy} onClick={() => openEditor({ action: "rename-window", target: window.target, value: window.name })}><Pencil size={13} /></button>
                        <button type="button" className="danger" title={t("close-window-3", [window.target])} aria-label={t("close-window-3", [window.target])} disabled={operationBusy} onClick={() => {
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
                            title={t("activate-pane", [paneLabel])}
                            aria-label={t("activate-pane", [paneLabel])}
                            disabled={operationBusy || !pane.paneId}
                            onClick={() => applyPaneMutation("select-pane", pane.paneId, paneLabel)}
                          >
                            <strong>{paneLabel}</strong>
                            <span>{pane.command || "shell"}</span>
                            <small>{pane.title || pane.paneId}</small>
                          </button>
                          <div className="tmux-pane-actions">
                          <select
                            aria-label={t("pane-split", [paneLabel])}
                            title={t("split", [paneLabel])}
                            defaultValue=""
                            disabled={operationBusy || !pane.paneId}
                            onChange={(event) => {
                              const action = event.currentTarget.value as TmuxPaneMutationAction;
                              event.currentTarget.value = "";
                              if (action) applyPaneMutation(action, pane.paneId, paneLabel);
                            }}
                          >
                            <option value="" disabled>{t("split-2")}</option>
                            <option value="split-pane-horizontal">{t("left-right")}</option>
                            <option value="split-pane-vertical">{t("top-bottom")}</option>
                          </select>
                          <select
                            aria-label={t("pane-swap", [paneLabel])}
                            title={t("swap", [paneLabel])}
                            defaultValue=""
                            disabled={operationBusy || !pane.paneId || window.panes.length < 2}
                            onChange={(event) => {
                              const action = event.currentTarget.value as TmuxPaneMutationAction;
                              event.currentTarget.value = "";
                              if (action) applyPaneMutation(action, pane.paneId, paneLabel);
                            }}
                          >
                            <option value="" disabled>{t("swap-2")}</option>
                            <option value="swap-pane-previous">{t("previous")}</option>
                            <option value="swap-pane-next">{t("next")}</option>
                          </select>
                          <select
                            aria-label={t("resize-pane", [paneLabel])}
                            title={t("resize", [paneLabel])}
                            defaultValue=""
                            disabled={operationBusy || !pane.paneId}
                            onChange={(event) => {
                              const action = event.currentTarget.value as TmuxPaneMutationAction;
                              event.currentTarget.value = "";
                              if (action) applyPaneMutation(action, pane.paneId, paneLabel);
                            }}
                          >
                            <option value="" disabled>{t("size-2")}</option>
                            <option value="resize-pane-left">{t("left-5")}</option>
                            <option value="resize-pane-right">{t("right-5")}</option>
                            <option value="resize-pane-up">{t("up-5")}</option>
                            <option value="resize-pane-down">{t("down-5")}</option>
                          </select>
                          <select
                            className="tmux-pane-move-select"
                            aria-label={t("move-pane", [paneLabel])}
                            title={t("move", [paneLabel])}
                            defaultValue=""
                            disabled={operationBusy || !pane.paneId || !moveOptions.length}
                            onChange={(event) => {
                              const option = moveOptions.find((item) => item.key === event.currentTarget.value);
                              event.currentTarget.value = "";
                              if (option) applyPaneMove(option, pane.paneId, paneLabel);
                            }}
                          >
                            <option value="" disabled>{t("move-2")}</option>
                            {moveOptions.map((option) => (
                              <option value={option.key} key={option.key}>{option.label}</option>
                            ))}
                          </select>
                          <button
                            type="button"
                            className="danger"
                            title={t("close-pane-2", [paneLabel])}
                            aria-label={t("close-pane-2", [paneLabel])}
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
              {!windows.length ? <div className="empty-pane top">{t("no-panes-available")}</div> : null}
            </div>
          </section>
        </div>
      </section>
    </div>
  );
}

function paneMutationFeedback(action: TmuxPaneMutationAction, paneLabel: string): string {
  switch (action) {
    case "select-pane": return t("activated", [paneLabel]);
    case "split-pane-horizontal": return t("split-left-right", [paneLabel]);
    case "split-pane-vertical": return t("split-top-bottom", [paneLabel]);
    case "swap-pane-previous": return t("swapped-with-the-previous-pane", [paneLabel]);
    case "swap-pane-next": return t("swapped-with-the-next-pane", [paneLabel]);
    case "resize-pane-left": return t("resized-5-cells-left", [paneLabel]);
    case "resize-pane-right": return t("resized-5-cells-right", [paneLabel]);
    case "resize-pane-up": return t("resized-5-cells-up", [paneLabel]);
    case "resize-pane-down": return t("resized-5-cells-down", [paneLabel]);
  }
}

function buildPaneMoveOptions(
  windows: readonly TmuxWindowGroup[],
  currentWindowTarget: string,
  allowBreak: boolean,
): TmuxPaneMoveOption[] {
  const options: TmuxPaneMoveOption[] = allowBreak
    ? [{ key: "break", label: t("split-into-a-new-window-2"), action: "break-pane" }]
    : [];
  for (const destination of windows) {
    if (destination.target === currentWindowTarget) continue;
    options.push(
      {
        key: `horizontal:${destination.target}`,
        label: t("left-right-2", [destination.target]),
        action: "move-pane-horizontal",
        destination: destination.target,
      },
      {
        key: `vertical:${destination.target}`,
        label: t("top-bottom-2", [destination.target]),
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
  useLocale();
  const rename = editor.action !== "new-window";
  return (
    <form className="tmux-inline-editor" onSubmit={(event) => {
      event.preventDefault();
      onSubmit();
    }}>
      <input
        autoFocus
        aria-label={editor.action === "new-window" ? t("new-window-name-for", [editor.target]) : t("new-name-for", [editor.target])}
        maxLength={128}
        placeholder={editor.action === "new-window" ? t("window-name-optional") : t("new-name")}
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
      <button type="submit" title={t("save")} aria-label={t("save")} disabled={busy || (rename && !editor.value.trim())}><Check size={14} /></button>
      <button type="button" title={t("cancel")} aria-label={t("cancel")} disabled={busy} onClick={onCancel}><X size={14} /></button>
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
