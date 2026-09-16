import { t, useLocale, localizeDiagnostic } from "./i18n";
import { useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import { CalendarClock, Copy, Dices, Download, KeyRound, ListX, Plus, RefreshCw, Search, Trash2, X } from "lucide-react";
import { invokeBackend, isBackendAvailable } from "./api";
import { KeyedRequestGate } from "./keyed-request-gate";
import { filterMcpAudit, MCP_AUDIT_GLOBAL_SESSION, mcpAuditDecisionOptions } from "./mcp-audit-state";
import { createMcpGrant, formatMcpGrantExpiryInput, generateMcpClientId, mcpGrantDraftHasUnsavedChanges, mcpGrantIsActive, mcpSessionAccessMode, parseMcpGrantExpiryInput, setMcpSessionAccessMode, MCP_NO_SESSIONS_SENTINEL } from "./mcp-grant-state";
import McpHttpPanel from "./McpHttpPanel";
import McpRevokeDialog from "./McpRevokeDialog";
import McpScopeEditor, { allMcpScopes } from "./McpScopeEditor";
import "./mcp-management.css";
import { useMcpHttpController } from "./use-mcp-http-controller";
import { mcpHttpRuntimeLabel } from "./mcp-http-workflow";
import type { AuditRecord, ExportMcpAuditResult, McpGrant, McpGrantMutationResponse, McpScope, SessionSummary } from "./types";

type McpDialogTab = "grants" | "http" | "audit";
type McpExpiryEditorState = { date: string; time: string };
type McpRevokeConfirmation = Pick<McpGrant, "clientId" | "name">;

export default function McpDialog({
  grants,
  audit,
  sessions,
  onClose,
  onGrantMutationStart,
  onGrantChange,
  onGrantMutationFinish,
  onAuditChange,
  initialTab = "grants",
}: {
  grants: McpGrant[];
  audit: AuditRecord[];
  sessions: SessionSummary[];
  onClose: () => void;
  onGrantMutationStart: () => number;
  onGrantChange: (grants: McpGrant[], token: number) => boolean;
  onGrantMutationFinish: (token: number) => void;
  onAuditChange: (audit: AuditRecord[]) => void;
  initialTab?: McpDialogTab;
}) {
  useLocale();
  const [tab, setTab] = useState<McpDialogTab>(initialTab);
  const [draft, setDraft] = useState<McpGrant | null>(() => grants[0] ?? null);
  const [editingClientId, setEditingClientId] = useState<string | null>(() => grants[0]?.clientId ?? null);
  const [creatingGrant, setCreatingGrant] = useState(false);
  const [expiryEditor, setExpiryEditor] = useState<McpExpiryEditorState | null>(null);
  const [error, setError] = useState("");
  const http = useMcpHttpController(grants);
  const [grantNotice, setGrantNotice] = useState("");
  const [revokeConfirmation, setRevokeConfirmation] = useState<McpRevokeConfirmation | null>(null);
  const [grantBusy, setGrantBusy] = useState(false);
  const [auditBusy, setAuditBusy] = useState(false);
  const [auditQuery, setAuditQuery] = useState("");
  const [auditDecision, setAuditDecision] = useState("");
  const [auditSessionId, setAuditSessionId] = useState("");
  const [auditScope, setAuditScope] = useState<"" | McpScope>("");
  const [selectedAuditId, setSelectedAuditId] = useState("");
  const [auditExport, setAuditExport] = useState<ExportMcpAuditResult | null>(null);
  const clientIdInputRef = useRef<HTMLInputElement>(null);
  const expiryEditorRef = useRef<HTMLDivElement>(null);
  const expiryDateInputRef = useRef<HTMLInputElement>(null);
  const loadedGrantRef = useRef<McpGrant | null>(grants[0] ?? null);
  const requestGateRef = useRef(new KeyedRequestGate<"grants" | "audit">());

  const filteredAudit = useMemo(() => filterMcpAudit(audit, {
    query: auditQuery,
    decision: auditDecision,
    sessionId: auditSessionId,
    scope: auditScope,
  }), [audit, auditDecision, auditQuery, auditScope, auditSessionId]);
  const selectedAudit = filteredAudit.find((record) => record.id === selectedAuditId) ?? filteredAudit[0] ?? null;
  const decisionOptions = useMemo(() => mcpAuditDecisionOptions(audit), [audit]);
  const sessionNames = useMemo(() => new Map(sessions.map((session) => [session.profile.id, session.profile.name])), [sessions]);
  const auditSessionOptions = useMemo(() => {
    const ids = new Set(audit.flatMap((record) => record.sessionId ? [record.sessionId] : []));
    return [...ids].sort((left, right) => (sessionNames.get(left) ?? left).localeCompare(sessionNames.get(right) ?? right));
  }, [audit, sessionNames]);
  const grantDirty = mcpGrantDraftHasUnsavedChanges(draft, loadedGrantRef.current);
  const expiryCandidate = expiryEditor
    ? parseMcpGrantExpiryInput(`${expiryEditor.date}T${expiryEditor.time}`)
    : null;
  const activeGrantCount = http.activeGrants.length;
  const runtimePhase = http.runtime?.phase ?? "stopped";
  const runtimeEndpoint = http.runtime?.endpoint ?? http.savedConfig?.clientEndpoint ?? "—";

  useEffect(() => {
    if (creatingGrant) return;
    const selected = editingClientId
      ? grants.find((grant) => grant.clientId === editingClientId)
      : undefined;
    const next = selected ?? grants[0] ?? null;
    if (selected && mcpGrantDraftHasUnsavedChanges(draft, loadedGrantRef.current)) return;
    loadedGrantRef.current = next;
    setDraft(next);
    setEditingClientId(next?.clientId ?? null);
  }, [creatingGrant, editingClientId, grants]);

  useEffect(() => () => requestGateRef.current.invalidateAll(), []);

  useEffect(() => {
    if (!expiryEditor) return;
    const closeOnOutsidePointer = (event: PointerEvent) => {
      if (event.target instanceof Node && !expiryEditorRef.current?.contains(event.target)) {
        setExpiryEditor(null);
      }
    };
    window.addEventListener("pointerdown", closeOnOutsidePointer);
    return () => window.removeEventListener("pointerdown", closeOnOutsidePointer);
  }, [Boolean(expiryEditor)]);

  async function saveGrant(openHttp = false) {
    if (!draft || grantBusy || http.isMutating()) return;
    const pendingGrant = draft;
    const token = requestGateRef.current.begin("grants");
    if (token === null) return;
    const mutationToken = onGrantMutationStart();
    setError("");
    setGrantNotice("");
    setGrantBusy(true);
    try {
      const result = await invokeBackend<McpGrantMutationResponse>("save_mcp_grant", { grant: pendingGrant });
      const accepted = onGrantChange(result.grants, mutationToken);
      if (!accepted || !requestGateRef.current.isCurrent("grants", token)) return;
      const selected = result.grants.find(grant => grant.clientId === pendingGrant.clientId) ?? null;
      loadedGrantRef.current = selected;
      setDraft(selected);
      setEditingClientId(selected?.clientId ?? null);
      setCreatingGrant(false);
      setError(result.warnings.join("\n"));
      setGrantNotice(t("grant-saved-select-this-client-on-the-http-page"));
      await http.refreshAfterGrantChange(result.httpAccessInvalidated);
      if (openHttp && requestGateRef.current.isCurrent("grants", token) && !result.warnings.length) setTab("http");
    } catch (cause) {
      if (requestGateRef.current.isCurrent("grants", token)) setError(formatError(cause));
    } finally {
      onGrantMutationFinish(mutationToken);
      if (requestGateRef.current.finish("grants", token)) setGrantBusy(false);
    }
  }

  function isHttpBindingClient(clientId: string): boolean {
    return http.savedConfig?.clientId === clientId;
  }

  function requestGrantRevocation(clientId: string) {
    if (grantBusy || http.isMutating()) return;
    const target = grants.find(grant => grant.clientId === clientId);
    if (!target) return;
    const unsavedWarning = grantDirty ? t("changes-to-the-current-grant-draft-will-be-discarded") : "";
    const bridgeWarning = isHttpBindingClient(clientId)
      ? t("this-identity-is-bound-to-http-bridge-the-managed")
      : "";
    if (!window.confirm(t("revoke-mcp-grant", [target.name || clientId, clientId, bridgeWarning, unsavedWarning]))) return;
    setError("");
    setRevokeConfirmation({ clientId, name: target.name });
  }

  async function revokeGrant() {
    const clientId = revokeConfirmation?.clientId;
    if (!clientId || grantBusy || http.isMutating()) return;
    const token = requestGateRef.current.begin("grants");
    if (token === null) return;
    const mutationToken = onGrantMutationStart();
    setError("");
    setGrantNotice("");
    setGrantBusy(true);
    try {
      const result = await invokeBackend<McpGrantMutationResponse>("revoke_mcp_grant", { clientId });
      const accepted = onGrantChange(result.grants, mutationToken);
      if (!accepted || !requestGateRef.current.isCurrent("grants", token)) return;
      setRevokeConfirmation(null);
      loadedGrantRef.current = result.grants[0] ?? null;
      setDraft(loadedGrantRef.current);
      setEditingClientId(loadedGrantRef.current?.clientId ?? null);
      setCreatingGrant(false);
      setError(result.warnings.join("\n"));
      setGrantNotice(result.httpAccessInvalidated
        ? t("grant-revoked-and-http-identity-invalidated-select-an-active")
        : t("grant-revoked-other-clients-and-http-settings-are-unchanged"));
      await http.refreshAfterGrantChange(result.httpAccessInvalidated);
    } catch (cause) {
      if (requestGateRef.current.isCurrent("grants", token)) setError(formatError(cause));
    } finally {
      onGrantMutationFinish(mutationToken);
      if (requestGateRef.current.finish("grants", token)) setGrantBusy(false);
    }
  }

  async function refreshAudit() {
    const token = requestGateRef.current.begin("audit");
    if (token === null) return;
    setError("");
    setAuditBusy(true);
    try {
      const next = await invokeBackend<AuditRecord[]>("list_mcp_audit", {});
      if (requestGateRef.current.isCurrent("audit", token)) onAuditChange(next);
    } catch (nextError) {
      if (requestGateRef.current.isCurrent("audit", token)) setError(formatError(nextError));
    } finally {
      if (requestGateRef.current.finish("audit", token)) setAuditBusy(false);
    }
  }

  async function exportAudit() {
    const token = requestGateRef.current.begin("audit");
    if (token === null) return;
    setError("");
    setAuditBusy(true);
    setAuditExport(null);
    try {
      const next = await invokeBackend<ExportMcpAuditResult>("export_mcp_audit", {
        request: { recordIds: filteredAudit.map((record) => record.id) },
      });
      if (requestGateRef.current.isCurrent("audit", token)) setAuditExport(next);
    } catch (nextError) {
      if (requestGateRef.current.isCurrent("audit", token)) setError(formatError(nextError));
    } finally {
      if (requestGateRef.current.finish("audit", token)) setAuditBusy(false);
    }
  }

  async function deleteAudit(recordIds: string[], all = false) {
    if (auditBusy || (!all && !recordIds.length)) return;
    const targetLabel = all
      ? t("all-mcp-audit-records")
      : recordIds.length === filteredAudit.length && recordIds.length === audit.length
        ? t("all-mcp-audit-records")
        : t("mcp-audit-records", [recordIds.length]);
    if (!window.confirm(t("delete-this-cannot-be-undone", [targetLabel]))) return;
    const token = requestGateRef.current.begin("audit");
    if (token === null) return;
    setError("");
    setAuditBusy(true);
    setAuditExport(null);
    try {
      const next = await invokeBackend<AuditRecord[]>("delete_mcp_audit", {
        request: all ? { all: true, recordIds: [] } : { all: false, recordIds },
      });
      if (requestGateRef.current.isCurrent("audit", token)) {
        onAuditChange(next);
        if (!next.some((record) => record.id === selectedAuditId)) setSelectedAuditId("");
      }
    } catch (nextError) {
      if (requestGateRef.current.isCurrent("audit", token)) setError(formatError(nextError));
    } finally {
      if (requestGateRef.current.finish("audit", token)) setAuditBusy(false);
    }
  }

  function selectGrant(grant: McpGrant) {
    if (grantBusy || !confirmDiscardGrant(t("switch-grant"))) return;
    setExpiryEditor(null);
    loadedGrantRef.current = grant;
    setGrantNotice("");
    setDraft(grant);
    setEditingClientId(grant.clientId);
    setCreatingGrant(false);
    setError("");
  }

  function newGrant() {
    if (grantBusy || !confirmDiscardGrant(t("new-grant"))) return;
    setExpiryEditor(null);
    loadedGrantRef.current = null;
    setGrantNotice("");
    setDraft(createMcpGrant());
    setEditingClientId(null);
    setCreatingGrant(true);
    setError("");
    requestAnimationFrame(() => {
      clientIdInputRef.current?.focus();
      clientIdInputRef.current?.select();
    });
  }

  function fillRandomClientId() {
    if (!draft || editingClientId !== null || grantBusy) return;
    try {
      let clientId = "";
      for (let attempt = 0; attempt < 8; attempt += 1) {
        clientId = generateMcpClientId();
        if (!grants.some((grant) => grant.clientId === clientId)) break;
      }
      setDraft({ ...draft, clientId });
      setError("");
      // Keep the editor focused even when the browser dispatches the button click
      // after a state update has replaced the controlled input value.
      clientIdInputRef.current?.focus();
      clientIdInputRef.current?.select();
      requestAnimationFrame(() => {
        clientIdInputRef.current?.focus();
        clientIdInputRef.current?.select();
      });
    } catch (nextError) {
      setError(formatError(nextError));
    }
  }

  function openExpiryEditor() {
    if (!draft || grantBusy) return;
    const current = formatMcpGrantExpiryInput(draft.expiresAt) || defaultMcpGrantExpiryInput();
    const [date = "", time = ""] = current.split("T");
    setExpiryEditor({ date, time });
    requestAnimationFrame(() => {
      expiryDateInputRef.current?.focus();
      expiryDateInputRef.current?.select();
    });
  }

  function applyExpiry() {
    if (!draft || !expiryCandidate || grantBusy) return;
    setDraft({ ...draft, expiresAt: expiryCandidate });
    setExpiryEditor(null);
    setError("");
  }

  function clearExpiry() {
    if (!draft || grantBusy) return;
    setDraft({ ...draft, expiresAt: null });
    setExpiryEditor(null);
    setError("");
  }

  function toggleScope(scope: McpScope) {
    if (grantBusy) return;
    setDraft((current) => current ? ({
      ...current,
      scopes: current.scopes.includes(scope) ? current.scopes.filter((item) => item !== scope) : [...current.scopes, scope],
    }) : current);
  }

  function toggleSession(sessionId: string) {
    if (grantBusy) return;
    setDraft((current) => current ? ({
      ...current,
      allowedSessions: (() => {
        const selected = current.allowedSessions.filter((item) => item !== MCP_NO_SESSIONS_SENTINEL);
        return selected.includes(sessionId)
          ? (selected.length === 1 ? [MCP_NO_SESSIONS_SENTINEL] : selected.filter((item) => item !== sessionId))
          : [...selected, sessionId];
      })(),
    }) : current);
  }

  function changeSessionAccessMode(mode: "none" | "all" | "selected") {
    if (grantBusy) return;
    setDraft((current) => {
      if (!current) return current;
      const next = setMcpSessionAccessMode(current, mode);
      if (mode === "selected" && mcpSessionAccessMode(next) === "none" && sessions[0]) {
        return { ...next, allowedSessions: [sessions[0].profile.id] };
      }
      return next;
    });
  }

  function confirmDiscardGrant(action: string): boolean {
    return !grantDirty || window.confirm(t("the-current-mcp-grant-has-unsaved-changes-will-discard", [action]));
  }

  function closeDialog() {
    const dirtySections = [grantDirty ? t("grant-draft") : "", http.dirty ? t("http-settings") : ""].filter(Boolean);
    if (dirtySections.length
      && !window.confirm(t("mcp-has-unsaved-changes-closing-will-discard-them-continue", [dirtySections.join(t("and"))]))) return;
    onClose();
  }

  return (
    <>
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && closeDialog()}>
      <section className="wind-dialog mcp-dialog" data-tab={tab} aria-label={t("mcp-bridge")}>
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>MCP Bridge</strong>
          <button type="button" title={t("close")} aria-label={t("close-mcp-bridge")} onClick={closeDialog}><X size={20} /></button>
        </header>
        <div className="mcp-overview" aria-label={t("mcp-bridge-current-status")}>
          <span className={`mcp-overview-service ${runtimePhase}`}><i aria-hidden="true" /><strong>{mcpHttpRuntimeLabel(http.runtime)}</strong></span>
          <code title={runtimeEndpoint}>{runtimeEndpoint}</code>
          <span>{t("active-grants", [activeGrantCount])}</span>
          <span>{t("audit-records", [audit.length])}</span>
        </div>
        <nav className="mcp-tabs" role="tablist" aria-label={t("mcp-bridge-views")}>
          {([['grants', t("grants")], ['http', 'HTTP'], ['audit', t("audit")]] as const).map(([id, label]) => (
            <button key={id} type="button" role="tab" aria-selected={tab === id} className={tab === id ? "active" : ""} onClick={() => { setTab(id); setError(""); }}>{label}</button>
          ))}
        </nav>

        {tab === "grants" ? (
          <div className="mcp-content" role="tabpanel">
            <aside className="mcp-grants">
              <header className="mcp-grants-header">
                <div><strong>{t("client-grants")}</strong><span>{t("active", [activeGrantCount])}</span></div>
                <button type="button" className="mcp-new" disabled={grantBusy} onClick={newGrant}><Plus size={14} />{t("new")}</button>
              </header>
              {draft && editingClientId === null ? (
                <button type="button" className="active mcp-grant-draft" aria-current="true" disabled={grantBusy} onClick={() => clientIdInputRef.current?.focus()}>
                  <strong>{draft.name.trim() || draft.clientId.trim() || t("new-grant-2")}</strong>
                  <span>{draft.clientId.trim() || t("not-yet-saved")}</span>
                </button>
              ) : null}
              {grants.map((grant) => (
                <button key={grant.clientId} type="button" disabled={grantBusy}
                  className={`mcp-grant-select ${grant.clientId === editingClientId ? "active" : ""}`}
                  onClick={() => selectGrant(grant)}>
                  <span className="mcp-grant-title"><strong>{grant.name || grant.clientId}</strong>{http.savedGrant?.clientId === grant.clientId ? <em>HTTP</em> : null}</span>
                  <span>{mcpGrantIsActive(grant) ? t("permissions-2", [grant.scopes.length, mcpSessionAccessMode(grant) === "none" ? t("no-session-access") : mcpSessionAccessMode(grant) === "all" ? t("all-sessions") : t("session-count", [grant.allowedSessions.length])]) : t("expired-or-revoked")}</span>
                </button>
              ))}
              {!grants.length && !draft ? <div className="empty-pane top">{t("no-grant-rules")}</div> : null}
            </aside>
            {draft ? (
              <section className="mcp-editor-shell">
                <div className="mcp-editor">
                <header className="mcp-section-heading"><div><strong>{editingClientId ? t("grant-details") : t("new-grant")}</strong><span>{http.savedGrant?.clientId === draft.clientId ? t("http-bridge-currently-uses-this-grant-manage-networking-and") : t("define-client-identity-permissions-and-accessible-sessions")}</span></div>{grantDirty ? <em>{t("not-saved")}</em> : null}</header>
                <McpFieldGroup label={t("ui-client-id")}>
                  <div className="mcp-client-id-control">
                    <input ref={clientIdInputRef} aria-label={t("mcp-grant-client-id")} value={draft.clientId} readOnly={editingClientId !== null} disabled={grantBusy} required maxLength={128} spellCheck={false} onChange={(event) => setDraft({ ...draft, clientId: event.target.value })} />
                    {editingClientId === null ? <button type="button" title={t("generate-random-client-id")} aria-label={t("generate-random-client-id")} disabled={grantBusy} onMouseDown={(event) => event.preventDefault()} onClick={fillRandomClientId}><Dices size={15} /></button> : null}
                  </div>
                </McpFieldGroup>
                <McpField label={t("name-2")}><input value={draft.name} disabled={grantBusy} maxLength={256} onChange={(event) => setDraft({ ...draft, name: event.target.value })} /></McpField>
                <McpFieldGroup label={t("expires")}>
                  <div className="mcp-expiry-control" ref={expiryEditorRef}>
                    <div className="mcp-expiry-summary">
                      <input type="text" readOnly disabled={grantBusy} aria-label={t("mcp-grant-expiration")} value={formatMcpGrantExpiryInput(draft.expiresAt)} placeholder={t("never-expires")} onClick={openExpiryEditor} onKeyDown={(event) => {
                        if (event.key === "Enter" || event.key === " ") {
                          event.preventDefault();
                          openExpiryEditor();
                        }
                      }} />
                      <button type="button" title={t("edit-expiration")} aria-label={t("edit-mcp-grant-expiration")} aria-expanded={Boolean(expiryEditor)} disabled={grantBusy} onClick={() => expiryEditor ? setExpiryEditor(null) : openExpiryEditor()}><CalendarClock size={15} /></button>
                    </div>
                    {expiryEditor ? (
                      <div className="mcp-expiry-editor" onKeyDown={(event) => {
                        if (event.key === "Escape") {
                          event.preventDefault();
                          event.stopPropagation();
                          setExpiryEditor(null);
                        } else if (event.key === "Enter" && expiryCandidate) {
                          event.preventDefault();
                          applyExpiry();
                        }
                      }}>
                        <label><span>{t("date")}</span><input ref={expiryDateInputRef} type="text" inputMode="numeric" aria-label={t("mcp-grant-expiration-date")} disabled={grantBusy} maxLength={10} placeholder="YYYY-MM-DD" value={expiryEditor.date} onChange={(event) => setExpiryEditor({ ...expiryEditor, date: event.target.value })} /></label>
                        <label><span>{t("time")}</span><input type="text" inputMode="numeric" aria-label={t("mcp-grant-expiration-time")} disabled={grantBusy} maxLength={5} placeholder="HH:mm" value={expiryEditor.time} onChange={(event) => setExpiryEditor({ ...expiryEditor, time: event.target.value })} /></label>
                        <small className={expiryEditor.date && expiryEditor.time && !expiryCandidate ? "invalid" : ""}>{expiryEditor.date && expiryEditor.time && !expiryCandidate ? t("enter-a-valid-local-date-and-time") : t("uses-the-local-time-zone")}</small>
                        <div className="mcp-expiry-actions">
                          <button type="button" disabled={grantBusy} onClick={clearExpiry}>{t("clear")}</button>
                          <button type="button" disabled={grantBusy} onClick={() => setExpiryEditor(null)}>{t("cancel")}</button>
                          <button type="button" className="primary" disabled={grantBusy || !expiryCandidate} onClick={applyExpiry}>{t("ok")}</button>
                        </div>
                      </div>
                    ) : null}
                  </div>
                </McpFieldGroup>
                <McpField label={t("write-actions")}><span className="mcp-confirm-write"><input type="checkbox" aria-label={t("confirm-each-write-action")} disabled={grantBusy} checked={Boolean(draft.confirmWrites)} onChange={(event) => setDraft({ ...draft, confirmWrites: event.target.checked })} />{t("confirm-each-time")}</span></McpField>
                <McpScopeEditor scopes={draft.scopes} disabled={grantBusy} onToggle={toggleScope} />
                <p className={draft.scopes.includes("host-files") ? "mcp-scope-boundary elevated" : "mcp-scope-boundary"}>
                  <code>transfer</code>{t("can-use-mcp-virtual-content-and")}<code>uploadId</code>；<code>host-files</code>{t("also-exposes-paths-on-the-portmate-host-only-grant")}</p>
                <p className="mcp-scope-boundary"><code>run-scripts</code>{t("executes-python-shell-scripts-explicitly-exposed-to-this-client")}</p>
                <fieldset className="mcp-session-list">
                  <legend>{t("allowed-sessions")}</legend>
                  <div className="mcp-session-access-mode" role="radiogroup" aria-label={t("mcp-session-access-scope")}>
                    {([["none", t("no-session-access")], ["all", t("all-sessions")], ["selected", t("selected-sessions-only")]] as const).map(([mode, label]) => (
                      <label key={mode}><input type="radio" name="mcp-session-access-mode" value={mode} disabled={grantBusy} checked={mcpSessionAccessMode(draft) === mode} onChange={() => changeSessionAccessMode(mode)} />{label}</label>
                    ))}
                  </div>
                  {mcpSessionAccessMode(draft) === "selected" ? (
                    <div className="mcp-session-checkboxes">
                      {sessions.map((session) => <label key={session.profile.id}><input type="checkbox" disabled={grantBusy} checked={draft.allowedSessions.includes(session.profile.id)} onChange={() => toggleSession(session.profile.id)} />{session.profile.name}</label>)}
                      {!sessions.length ? <span className="mcp-session-empty">{t("no-sessions-available")}</span> : null}
                    </div>
                  ) : (
                    <small className="mcp-session-access-hint">{mcpSessionAccessMode(draft) === "none" ? t("no-session-access-is-granted-by-default") : t("allows-access-to-all-sessions-currently-in-portmate")}</small>
                  )}
                </fieldset>
                {grantNotice ? <p className="mcp-inline-notice" role="status">{grantNotice}</p> : null}
                {error ? <div className="utility-error">{localizeDiagnostic(error)}</div> : null}
                <p className="mcp-bridge-binding-note">{t("permission-changes-take-effect-after-saving-manage-the-address")}</p>
                </div>
                <div className="mcp-actions mcp-grant-actions">
                  <button type="button" className="danger" onClick={() => requestGrantRevocation(draft.clientId)} disabled={grantBusy || http.busy || http.loading || !editingClientId}>{t("revoke")}</button>
                  <button type="button" disabled={grantBusy || http.busy || http.loading || !draft.clientId.trim()} onClick={() => void saveGrant()}>{t("save")}</button>
                  <button type="button" className="primary" disabled={grantBusy || http.busy || http.loading || !draft.clientId.trim()} onClick={() => grantDirty ? void saveGrant(true) : setTab("http")}>{grantDirty ? t("save-and-open-http") : t("open-http-access")}</button>
                </div>
              </section>
            ) : (
              <section className="mcp-editor mcp-editor-empty">
                <KeyRound size={22} aria-hidden="true" />
                <strong>{t("no-grant-selected")}</strong>
                {grantNotice ? <p className="mcp-inline-notice" role="status">{grantNotice}</p> : null}
                {error ? <div className="utility-error" role="alert">{localizeDiagnostic(error)}</div> : null}
                <button type="button" disabled={grantBusy} onClick={newGrant}><Plus size={14} />{t("new-grant")}</button>
              </section>
            )}
          </div>
        ) : null}

        {tab === "http" ? <McpHttpPanel http={http} grantBusy={grantBusy} onManageGrants={() => setTab("grants")} /> : null}

        {tab === "audit" ? (
          <section className="mcp-audit-view" role="tabpanel">
            <div className="mcp-audit-toolbar">
              <label className="mcp-audit-search"><Search size={14} /><input aria-label={t("filter-mcp-audit")} value={auditQuery} onChange={(event) => setAuditQuery(event.target.value)} placeholder={t("client-action-or-details")} /></label>
              <select aria-label={t("filter-audit-decisions")} value={auditDecision} onChange={(event) => setAuditDecision(event.target.value)}><option value="">{t("all-decisions")}</option>{decisionOptions.map((decision) => <option key={decision} value={decision}>{decision}</option>)}</select>
              <select aria-label={t("filter-audit-sessions")} value={auditSessionId} onChange={(event) => setAuditSessionId(event.target.value)}><option value="">{t("all-sessions")}</option><option value={MCP_AUDIT_GLOBAL_SESSION}>{t("global")}</option>{auditSessionOptions.map((id) => <option key={id} value={id}>{sessionNames.get(id) ?? id}</option>)}</select>
              <select aria-label={t("filter-audit-permissions")} value={auditScope} onChange={(event) => setAuditScope(event.target.value as "" | McpScope)}><option value="">{t("all-scopes")}</option>{allMcpScopes.map((scope) => <option key={scope} value={scope}>{scope}</option>)}</select>
              <span className="mcp-audit-count">{filteredAudit.length} / {audit.length}</span>
              <button type="button" className="icon-button" title={t("refresh-audit")} aria-label={t("refresh-mcp-audit")} disabled={auditBusy || !isBackendAvailable()} onClick={() => void refreshAudit()}><RefreshCw size={14} /></button>
              <button type="button" className="icon-button" title={t("export-filtered-results")} aria-label={t("export-mcp-audit")} disabled={auditBusy || !filteredAudit.length || !isBackendAvailable()} onClick={() => void exportAudit()}><Download size={14} /></button>
              <button type="button" className="icon-button danger" title={t("delete-current-audit-records")} aria-label={t("delete-current-mcp-audit")} disabled={auditBusy || !selectedAudit || !isBackendAvailable()} onClick={() => void deleteAudit(selectedAudit ? [selectedAudit.id] : [])}><Trash2 size={14} /></button>
              <button type="button" className="icon-button danger" title={filteredAudit.length === audit.length ? t("clear-all-audit-records") : t("delete-filtered-results")} aria-label={filteredAudit.length === audit.length ? t("clear-all-mcp-audit-records") : t("delete-filtered-results")} disabled={auditBusy || !filteredAudit.length || !isBackendAvailable()} onClick={() => void deleteAudit(filteredAudit.map((record) => record.id), filteredAudit.length === audit.length)}><ListX size={14} /></button>
            </div>
            {auditExport ? <div className="mcp-audit-export"><span>{t("exported-records-sha-256", [auditExport.records, auditExport.sha256.slice(0, 12)])}</span><button type="button" title={t("copy-export-details")} aria-label={t("copy-mcp-audit-export-details")} onClick={() => void navigator.clipboard?.writeText(`${auditExport.path}\n${auditExport.checksumPath}\nSHA-256 ${auditExport.sha256}`).catch(() => {})}><Copy size={14} /></button></div> : null}
            {error ? <div className="utility-error">{localizeDiagnostic(error)}</div> : null}
            <div className="mcp-audit-workspace">
              <div className="mcp-audit-list" role="listbox" aria-label={t("mcp-audit-records-2")}>
                {filteredAudit.map((record) => (
                  <button key={record.id} type="button" role="option" aria-selected={record.id === selectedAudit?.id} className={record.id === selectedAudit?.id ? "active" : ""} onClick={() => setSelectedAuditId(record.id)}>
                    <span><strong>{record.action}</strong><time>{formatDateTime(record.ts)}</time></span>
                    <span><code>{record.actor}</code><em className={`decision-${record.decision}`}>{record.decision}</em></span>
                    <small>{record.sessionId ? sessionNames.get(record.sessionId) ?? record.sessionId : t("global")} · {record.details.scope ?? "scope unknown"}</small>
                  </button>
                ))}
                {!filteredAudit.length ? <div className="empty-pane top">{t("no-matching-audit-records")}</div> : null}
              </div>
              <div className="mcp-audit-inspector">
                {selectedAudit ? <AuditInspector record={selectedAudit} sessionName={selectedAudit.sessionId ? sessionNames.get(selectedAudit.sessionId) : undefined} /> : <div className="empty-pane top">{t("select-an-audit-record")}</div>}
              </div>
            </div>
          </section>
        ) : null}
      </section>
    </div>
    {revokeConfirmation ? <McpRevokeDialog key={revokeConfirmation.clientId} target={revokeConfirmation}
      httpBound={isHttpBindingClient(revokeConfirmation.clientId)} busy={grantBusy || http.busy} error={error}
      onCancel={() => { setRevokeConfirmation(null); requestAnimationFrame(() => clientIdInputRef.current?.focus()); }}
      onConfirm={() => void revokeGrant()} /> : null}
    </>
  );
}

function AuditInspector({ record, sessionName }: { record: AuditRecord; sessionName?: string }) {
  useLocale();
  return (
    <>
      <header><strong>{record.action}</strong><span>{record.decision}</span></header>
      <dl>
        <div><dt>{t("time")}</dt><dd>{formatDateTime(record.ts)}</dd></div>
        <div><dt>{t("ui-client")}</dt><dd><code>{record.actor}</code></dd></div>
        <div><dt>{t("session")}</dt><dd>{record.sessionId ? <><span>{sessionName ?? record.sessionId}</span><code>{record.sessionId}</code></> : t("global")}</dd></div>
        <div><dt>{t("record-id")}</dt><dd><code>{record.id}</code></dd></div>
        {Object.entries(record.details).sort(([left], [right]) => left.localeCompare(right)).map(([key, value]) => <div key={key}><dt>{key}</dt><dd><code>{value}</code></dd></div>)}
      </dl>
    </>
  );
}

function McpField({ label, children }: { label: string; children: ReactNode }) {
  useLocale();
  return <label className="dialog-field"><span>{label}</span>{children}</label>;
}

function McpFieldGroup({ label, children }: { label: string; children: ReactNode }) {
  useLocale();
  return <div className="dialog-field"><span>{label}</span>{children}</div>;
}


function defaultMcpGrantExpiryInput() {
  const date = new Date(Date.now() + 30 * 24 * 60 * 60 * 1_000);
  date.setSeconds(0, 0);
  return formatMcpGrantExpiryInput(date.toISOString());
}

function formatDateTime(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function formatError(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
