import { useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import { CalendarClock, Copy, Dices, Download, KeyRound, Play, Plus, RefreshCw, Save, Search, Square, X } from "lucide-react";
import { invokeBackend, isBackendAvailable } from "./api";
import { KeyedRequestGate } from "./keyed-request-gate";
import { filterMcpAudit, MCP_AUDIT_GLOBAL_SESSION, mcpAuditDecisionOptions } from "./mcp-audit-state";
import { createMcpGrant, formatMcpGrantExpiryInput, generateMcpClientId, mcpGrantDraftHasUnsavedChanges, parseMcpGrantExpiryInput } from "./mcp-grant-state";
import {
  CC_SWITCH_DEFAULT_SERVER_ID,
  CC_SWITCH_DEFAULT_TOOL_TIMEOUT_SECONDS,
  defaultMcpHttpSettings,
  formatCcSwitchMcpJson,
  formatMcpHttpOrigins,
  isNonLoopbackMcpHost,
  MCP_HTTP_CUSTOM_LISTEN_PRESET,
  mcpHttpClientEndpoint,
  mcpHttpListenPreset,
  mcpHttpSettingsFromConfig,
  parseMcpHttpOrigins,
} from "./mcp-http-state";
import type { AuditRecord, ExportMcpAuditResult, McpGrant, McpHttpConfig, McpHttpConfigRequest, McpHttpRuntimeStatus, McpHttpTokenResponse, McpScope, SessionSummary } from "./types";

const allMcpScopes: McpScope[] = ["read-sessions", "read-logs", "read-transfers", "read-tunnels", "read-scripts", "write-input", "transfer", "tunnel", "manage-sessions", "run-scripts"];
const mcpHttpListenOptions = [
  ["127.0.0.1", "本机 IPv4 · 127.0.0.1"],
  ["0.0.0.0", "所有 IPv4 · 0.0.0.0"],
  ["::1", "本机 IPv6 · ::1"],
  ["::", "所有 IPv6 · ::"],
  [MCP_HTTP_CUSTOM_LISTEN_PRESET, "自定义 IP"],
] as const;
type McpDialogTab = "grants" | "http" | "audit";
type McpExpiryEditorState = { date: string; time: string };

export default function McpDialog({
  grants,
  audit,
  sessions,
  onClose,
  onGrantMutationStart,
  onGrantChange,
  onGrantMutationFinish,
  onAuditChange,
}: {
  grants: McpGrant[];
  audit: AuditRecord[];
  sessions: SessionSummary[];
  onClose: () => void;
  onGrantMutationStart: () => number;
  onGrantChange: (grants: McpGrant[], token: number) => boolean;
  onGrantMutationFinish: (token: number) => void;
  onAuditChange: (audit: AuditRecord[]) => void;
}) {
  const [tab, setTab] = useState<McpDialogTab>("grants");
  const [draft, setDraft] = useState<McpGrant | null>(() => grants[0] ?? null);
  const [editingClientId, setEditingClientId] = useState<string | null>(() => grants[0]?.clientId ?? null);
  const [creatingGrant, setCreatingGrant] = useState(false);
  const [expiryEditor, setExpiryEditor] = useState<McpExpiryEditorState | null>(null);
  const [error, setError] = useState("");
  const [httpConfig, setHttpConfig] = useState<McpHttpConfig | null>(null);
  const [httpRuntime, setHttpRuntime] = useState<McpHttpRuntimeStatus | null>(null);
  const [httpSettings, setHttpSettings] = useState<McpHttpConfigRequest>(defaultMcpHttpSettings);
  const [httpOriginsText, setHttpOriginsText] = useState(() => formatMcpHttpOrigins(defaultMcpHttpSettings().allowedOrigins));
  const [httpDirty, setHttpDirty] = useState(false);
  const [httpPreviewCurrent, setHttpPreviewCurrent] = useState(false);
  const [httpToken, setHttpToken] = useState("");
  const [httpBusy, setHttpBusy] = useState(false);
  const [httpRuntimeBusy, setHttpRuntimeBusy] = useState(false);
  const [ccSwitchServerId, setCcSwitchServerId] = useState(CC_SWITCH_DEFAULT_SERVER_ID);
  const [ccSwitchToolTimeout, setCcSwitchToolTimeout] = useState(CC_SWITCH_DEFAULT_TOOL_TIMEOUT_SECONDS);
  const [ccSwitchCopied, setCcSwitchCopied] = useState(false);
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
  const httpListenInputRef = useRef<HTMLInputElement>(null);
  const activeTabRef = useRef(tab);
  const httpRuntimeActionRef = useRef(false);
  const requestGateRef = useRef(new KeyedRequestGate<"grants" | "http" | "http-preview" | "http-runtime-status" | "http-runtime-action" | "audit">());
  activeTabRef.current = tab;

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
  const httpRemoteListener = isNonLoopbackMcpHost(httpSettings.listenHost);
  const httpRuntimeActive = httpRuntime?.phase === "starting" || httpRuntime?.phase === "running";
  const httpRuntimeLocked = httpRuntimeBusy || httpRuntimeActive;
  const httpControlsLocked = httpBusy || httpRuntimeLocked;
  const savedDraftGrant = editingClientId ? grants.find((grant) => grant.clientId === editingClientId) : null;
  const grantDirty = mcpGrantDraftHasUnsavedChanges(draft, savedDraftGrant);
  const expiryCandidate = expiryEditor
    ? parseMcpGrantExpiryInput(`${expiryEditor.date}T${expiryEditor.time}`)
    : null;
  const httpSettingsValid = Boolean(httpSettings.listenHost.trim() && httpSettings.clientId.trim())
    && Number.isInteger(httpSettings.port) && httpSettings.port >= 1 && httpSettings.port <= 65_535
    && Boolean(mcpHttpClientEndpoint(httpSettings))
    && (!httpRemoteListener || httpSettings.allowRemote);
  const ccSwitchJson = useMemo(() => formatCcSwitchMcpJson(httpSettings, {
    serverId: ccSwitchServerId,
    token: httpToken,
    toolTimeoutSeconds: ccSwitchToolTimeout,
  }), [ccSwitchServerId, ccSwitchToolTimeout, httpSettings, httpToken]);

  useEffect(() => {
    if (tab !== "http" || httpConfig || !isBackendAvailable()) return;
    void loadHttpConfig();
  }, [httpConfig, tab]);

  useEffect(() => {
    if (tab !== "http") {
      requestGateRef.current.invalidate("http-runtime-status");
      return;
    }
    if (isBackendAvailable()) void loadHttpRuntime();
  }, [tab]);

  useEffect(() => {
    if (tab !== "http" || !httpRuntimeActive || !isBackendAvailable()) return;
    const timer = window.setInterval(() => void loadHttpRuntime(), 1_000);
    return () => window.clearInterval(timer);
  }, [httpRuntimeActive, tab]);

  useEffect(() => {
    if (creatingGrant) return;
    const selected = editingClientId
      ? grants.find((grant) => grant.clientId === editingClientId)
      : undefined;
    const next = selected ?? grants[0] ?? null;
    setDraft(next);
    setEditingClientId(next?.clientId ?? null);
  }, [creatingGrant, editingClientId, grants]);

  useEffect(() => {
    if (tab !== "http") {
      requestGateRef.current.invalidate("http-preview");
      return;
    }
    if (!httpDirty || httpBusy || !isBackendAvailable()) return;
    if (!httpSettingsValid) {
      requestGateRef.current.invalidate("http-preview");
      setHttpPreviewCurrent(false);
      return;
    }
    const settings = currentHttpSettings();
    const timer = window.setTimeout(() => void previewHttpSettings(settings), 180);
    return () => window.clearTimeout(timer);
  }, [httpBusy, httpDirty, httpOriginsText, httpSettings, httpSettingsValid, tab]);

  useEffect(() => () => requestGateRef.current.invalidateAll(), []);

  useEffect(() => setCcSwitchCopied(false), [ccSwitchJson]);

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

  async function loadHttpConfig() {
    const token = requestGateRef.current.begin("http");
    if (token === null) return;
    setHttpBusy(true);
    try {
      const next = await invokeBackend<McpHttpConfig>("mcp_http_config", {});
      if (requestGateRef.current.isCurrent("http", token)) applyHttpConfig(next);
    } catch (nextError) {
      if (requestGateRef.current.isCurrent("http", token)) setError(formatError(nextError));
    } finally {
      if (requestGateRef.current.finish("http", token)) setHttpBusy(false);
    }
  }

  async function saveHttpSettings() {
    requestGateRef.current.invalidate("http-preview");
    const token = requestGateRef.current.begin("http");
    if (token === null) return;
    setError("");
    setHttpBusy(true);
    try {
      const next = await invokeBackend<McpHttpConfig>("save_mcp_http_settings", {
        settings: currentHttpSettings(),
      });
      if (requestGateRef.current.isCurrent("http", token)) applyHttpConfig(next);
    } catch (nextError) {
      if (requestGateRef.current.isCurrent("http", token)) setError(formatError(nextError));
    } finally {
      if (requestGateRef.current.finish("http", token)) setHttpBusy(false);
    }
  }

  async function loadHttpRuntime() {
    if (httpRuntimeActionRef.current) return;
    const token = requestGateRef.current.begin("http-runtime-status");
    if (token === null) return;
    try {
      const next = await invokeBackend<McpHttpRuntimeStatus>("mcp_http_runtime_status", {});
      if (requestGateRef.current.isCurrent("http-runtime-status", token)) setHttpRuntime(next);
    } catch (nextError) {
      if (requestGateRef.current.isCurrent("http-runtime-status", token)) setError(formatError(nextError));
    } finally {
      requestGateRef.current.finish("http-runtime-status", token);
    }
  }

  async function startHttpRuntime() {
    const token = requestGateRef.current.begin("http-runtime-action");
    if (token === null) return;
    httpRuntimeActionRef.current = true;
    requestGateRef.current.invalidate("http-runtime-status");
    setError("");
    setHttpRuntimeBusy(true);
    try {
      const next = await invokeBackend<McpHttpRuntimeStatus>("start_mcp_http", {});
      if (requestGateRef.current.isCurrent("http-runtime-action", token)) setHttpRuntime(next);
    } catch (nextError) {
      if (requestGateRef.current.isCurrent("http-runtime-action", token)) setError(formatError(nextError));
    } finally {
      if (requestGateRef.current.finish("http-runtime-action", token)) {
        httpRuntimeActionRef.current = false;
        requestGateRef.current.invalidate("http-runtime-status");
        setHttpRuntimeBusy(false);
        if (activeTabRef.current === "http" && isBackendAvailable()) void loadHttpRuntime();
      }
    }
  }

  async function stopHttpRuntime() {
    const token = requestGateRef.current.begin("http-runtime-action");
    if (token === null) return;
    httpRuntimeActionRef.current = true;
    requestGateRef.current.invalidate("http-runtime-status");
    setError("");
    setHttpRuntimeBusy(true);
    try {
      const next = await invokeBackend<McpHttpRuntimeStatus>("stop_mcp_http", {});
      if (requestGateRef.current.isCurrent("http-runtime-action", token)) setHttpRuntime(next);
    } catch (nextError) {
      if (requestGateRef.current.isCurrent("http-runtime-action", token)) setError(formatError(nextError));
    } finally {
      if (requestGateRef.current.finish("http-runtime-action", token)) {
        httpRuntimeActionRef.current = false;
        requestGateRef.current.invalidate("http-runtime-status");
        setHttpRuntimeBusy(false);
        if (activeTabRef.current === "http" && isBackendAvailable()) void loadHttpRuntime();
      }
    }
  }

  async function previewHttpSettings(settings: McpHttpConfigRequest) {
    const token = requestGateRef.current.replace("http-preview");
    try {
      const next = await invokeBackend<McpHttpConfig>("preview_mcp_http_config", { settings });
      if (requestGateRef.current.isCurrent("http-preview", token)) {
        setHttpConfig(next);
        setHttpPreviewCurrent(true);
        setError("");
      }
    } catch (nextError) {
      if (requestGateRef.current.isCurrent("http-preview", token)) {
        setHttpPreviewCurrent(false);
        setError(formatError(nextError));
      }
    } finally {
      requestGateRef.current.finish("http-preview", token);
    }
  }

  async function rotateHttpToken() {
    const token = requestGateRef.current.begin("http");
    if (token === null) return;
    setError("");
    setHttpBusy(true);
    try {
      const response = await invokeBackend<McpHttpTokenResponse>("rotate_mcp_http_token", {});
      if (requestGateRef.current.isCurrent("http", token)) {
        applyHttpConfig(response.config);
        setHttpToken(response.token);
      }
    } catch (nextError) {
      if (requestGateRef.current.isCurrent("http", token)) setError(formatError(nextError));
    } finally {
      if (requestGateRef.current.finish("http", token)) setHttpBusy(false);
    }
  }

  function applyHttpConfig(config: McpHttpConfig) {
    const settings = mcpHttpSettingsFromConfig(config);
    setHttpConfig(config);
    setHttpSettings(settings);
    setHttpOriginsText(formatMcpHttpOrigins(settings.allowedOrigins));
    setHttpDirty(false);
    setHttpPreviewCurrent(true);
    setError("");
  }

  function currentHttpSettings(): McpHttpConfigRequest {
    return { ...httpSettings, allowedOrigins: parseMcpHttpOrigins(httpOriginsText) };
  }

  function updateHttpSettings(patch: Partial<McpHttpConfigRequest>) {
    requestGateRef.current.invalidate("http-preview");
    setHttpSettings((current) => ({ ...current, ...patch }));
    setHttpDirty(true);
    setHttpPreviewCurrent(false);
    setError("");
  }

  function updateHttpListenHost(listenHost: string) {
    updateHttpSettings({
      listenHost,
      ...(!isNonLoopbackMcpHost(listenHost) ? { allowRemote: false } : {}),
    });
  }

  async function copyCcSwitchJson() {
    if (!ccSwitchJson) return;
    try {
      await navigator.clipboard.writeText(ccSwitchJson);
      setCcSwitchCopied(true);
      setError("");
    } catch (nextError) {
      setError(formatError(nextError));
    }
  }

  async function saveGrant() {
    if (!draft || grantBusy) return;
    const pendingGrant = draft;
    const token = requestGateRef.current.begin("grants");
    if (token === null) return;
    const mutationToken = onGrantMutationStart();
    setError("");
    setGrantBusy(true);
    try {
      const saved = await invokeBackend<McpGrant[]>("save_mcp_grant", { grant: pendingGrant });
      const accepted = onGrantChange(saved, mutationToken);
      if (!accepted || !requestGateRef.current.isCurrent("grants", token)) return;
      const selected = saved.find((grant) => grant.clientId === pendingGrant.clientId);
      if (selected) {
        setDraft(selected);
        setEditingClientId(selected.clientId);
        setCreatingGrant(false);
      }
    } catch (nextError) {
      if (requestGateRef.current.isCurrent("grants", token)) setError(formatError(nextError));
    } finally {
      onGrantMutationFinish(mutationToken);
      if (requestGateRef.current.finish("grants", token)) setGrantBusy(false);
    }
  }

  async function revokeGrant(clientId: string) {
    if (grantBusy) return;
    const saved = grants.find((grant) => grant.clientId === clientId);
    const label = saved?.name.trim() || clientId;
    const unsavedWarning = grantDirty ? "\n\n当前授权编辑器还有未保存的更改，也会一并丢弃。" : "";
    if (!window.confirm(`撤销 MCP 授权“${label}”（${clientId}）？${unsavedWarning}`)) return;
    const token = requestGateRef.current.begin("grants");
    if (token === null) return;
    const mutationToken = onGrantMutationStart();
    setError("");
    setGrantBusy(true);
    try {
      const saved = await invokeBackend<McpGrant[]>("revoke_mcp_grant", { clientId });
      const accepted = onGrantChange(saved, mutationToken);
      if (!accepted || !requestGateRef.current.isCurrent("grants", token)) return;
      setDraft(saved[0] ?? null);
      setEditingClientId(saved[0]?.clientId ?? null);
      setCreatingGrant(false);
    } catch (nextError) {
      if (requestGateRef.current.isCurrent("grants", token)) setError(formatError(nextError));
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

  function selectGrant(grant: McpGrant) {
    if (grantBusy || !confirmDiscardGrant("切换授权")) return;
    setExpiryEditor(null);
    setDraft(grant);
    setEditingClientId(grant.clientId);
    setCreatingGrant(false);
    setError("");
  }

  function newGrant() {
    if (grantBusy || !confirmDiscardGrant("新建授权")) return;
    setExpiryEditor(null);
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
      allowedSessions: current.allowedSessions.includes(sessionId) ? current.allowedSessions.filter((item) => item !== sessionId) : [...current.allowedSessions, sessionId],
    }) : current);
  }

  function confirmDiscardGrant(action: string): boolean {
    return !grantDirty || window.confirm(`当前 MCP 授权有未保存的更改，${action}将放弃这些内容。是否继续？`);
  }

  function closeDialog() {
    const dirtySections = [grantDirty ? "授权草稿" : "", httpDirty ? "HTTP 配置" : ""].filter(Boolean);
    if (dirtySections.length
      && !window.confirm(`MCP ${dirtySections.join("和")}尚未保存，关闭窗口将放弃这些内容。是否继续？`)) return;
    onClose();
  }

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && closeDialog()}>
      <section className="wind-dialog mcp-dialog" data-tab={tab} aria-label="MCP Bridge">
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>MCP Bridge</strong>
          <button type="button" title="关闭" aria-label="关闭 MCP Bridge" onClick={closeDialog}><X size={20} /></button>
        </header>
        <nav className="mcp-tabs" role="tablist" aria-label="MCP Bridge 视图">
          {([['grants', '授权'], ['http', 'HTTP'], ['audit', '审计']] as const).map(([id, label]) => (
            <button key={id} type="button" role="tab" aria-selected={tab === id} className={tab === id ? "active" : ""} onClick={() => { setTab(id); setError(""); }}>{label}</button>
          ))}
        </nav>

        {tab === "grants" ? (
          <div className="mcp-content" role="tabpanel">
            <aside className="mcp-grants">
              <button type="button" className="mcp-new" disabled={grantBusy} onClick={newGrant}><Plus size={14} />新建授权</button>
              {draft && editingClientId === null ? (
                <button type="button" className="active mcp-grant-draft" aria-current="true" disabled={grantBusy} onClick={() => clientIdInputRef.current?.focus()}>
                  <strong>{draft.name.trim() || draft.clientId.trim() || "新授权"}</strong>
                  <span>{draft.clientId.trim() || "尚未保存"}</span>
                </button>
              ) : null}
              {grants.map((grant) => (
                <button key={grant.clientId} type="button" disabled={grantBusy} className={grant.clientId === editingClientId ? "active" : ""} onClick={() => selectGrant(grant)}>
                  <strong>{grant.name || grant.clientId}</strong>
                  <span>{grant.scopes.join(", ") || "read-only"}</span>
                </button>
              ))}
              {!grants.length && !draft ? <div className="empty-pane top">没有授权规则</div> : null}
            </aside>
            {draft ? (
              <section className="mcp-editor">
                <McpFieldGroup label="Client ID:">
                  <div className="mcp-client-id-control">
                    <input ref={clientIdInputRef} aria-label="MCP 授权 Client ID" value={draft.clientId} readOnly={editingClientId !== null} disabled={grantBusy} required maxLength={128} spellCheck={false} onChange={(event) => setDraft({ ...draft, clientId: event.target.value })} />
                    {editingClientId === null ? <button type="button" title="随机生成 Client ID" aria-label="随机生成 Client ID" disabled={grantBusy} onMouseDown={(event) => event.preventDefault()} onClick={fillRandomClientId}><Dices size={15} /></button> : null}
                  </div>
                </McpFieldGroup>
                <McpField label="名称:"><input value={draft.name} disabled={grantBusy} maxLength={256} onChange={(event) => setDraft({ ...draft, name: event.target.value })} /></McpField>
                <McpFieldGroup label="到期时间:">
                  <div className="mcp-expiry-control" ref={expiryEditorRef}>
                    <div className="mcp-expiry-summary">
                      <input type="text" readOnly disabled={grantBusy} aria-label="MCP 授权到期时间" value={formatMcpGrantExpiryInput(draft.expiresAt)} placeholder="永不过期" onClick={openExpiryEditor} onKeyDown={(event) => {
                        if (event.key === "Enter" || event.key === " ") {
                          event.preventDefault();
                          openExpiryEditor();
                        }
                      }} />
                      <button type="button" title="编辑到期时间" aria-label="编辑 MCP 授权到期时间" aria-expanded={Boolean(expiryEditor)} disabled={grantBusy} onClick={() => expiryEditor ? setExpiryEditor(null) : openExpiryEditor()}><CalendarClock size={15} /></button>
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
                        <label><span>日期</span><input ref={expiryDateInputRef} type="text" inputMode="numeric" aria-label="MCP 授权到期日期" disabled={grantBusy} maxLength={10} placeholder="YYYY-MM-DD" value={expiryEditor.date} onChange={(event) => setExpiryEditor({ ...expiryEditor, date: event.target.value })} /></label>
                        <label><span>时间</span><input type="text" inputMode="numeric" aria-label="MCP 授权到期时刻" disabled={grantBusy} maxLength={5} placeholder="HH:mm" value={expiryEditor.time} onChange={(event) => setExpiryEditor({ ...expiryEditor, time: event.target.value })} /></label>
                        <small className={expiryEditor.date && expiryEditor.time && !expiryCandidate ? "invalid" : ""}>{expiryEditor.date && expiryEditor.time && !expiryCandidate ? "请输入有效的本地日期和时间" : "使用本机时区"}</small>
                        <div className="mcp-expiry-actions">
                          <button type="button" disabled={grantBusy} onClick={clearExpiry}>清除</button>
                          <button type="button" disabled={grantBusy} onClick={() => setExpiryEditor(null)}>取消</button>
                          <button type="button" className="primary" disabled={grantBusy || !expiryCandidate} onClick={applyExpiry}>确定</button>
                        </div>
                      </div>
                    ) : null}
                  </div>
                </McpFieldGroup>
                <McpField label="写操作:"><span className="mcp-confirm-write"><input type="checkbox" aria-label="写操作每次确认" disabled={grantBusy} checked={Boolean(draft.confirmWrites)} onChange={(event) => setDraft({ ...draft, confirmWrites: event.target.checked })} />每次确认</span></McpField>
                <fieldset className="mcp-check-grid">
                  <legend>权限范围</legend>
                  {allMcpScopes.map((scope) => <label key={scope}><input type="checkbox" disabled={grantBusy} checked={draft.scopes.includes(scope)} onChange={() => toggleScope(scope)} />{scope}</label>)}
                </fieldset>
                <fieldset className="mcp-session-list">
                  <legend>允许会话 · {draft.allowedSessions.length ? `${draft.allowedSessions.length} 个` : "全部"}</legend>
                  {sessions.map((session) => <label key={session.profile.id}><input type="checkbox" disabled={grantBusy} checked={draft.allowedSessions.includes(session.profile.id)} onChange={() => toggleSession(session.profile.id)} />{session.profile.name}</label>)}
                </fieldset>
                {error ? <div className="utility-error">{error}</div> : null}
                <div className="mcp-actions">
                  <button type="button" disabled={grantBusy || !draft.clientId.trim()} onClick={() => void saveGrant()}>保存</button>
                  <button type="button" onClick={() => void revokeGrant(draft.clientId)} disabled={grantBusy || !editingClientId}>撤销</button>
                </div>
              </section>
            ) : (
              <section className="mcp-editor mcp-editor-empty">
                <KeyRound size={22} aria-hidden="true" />
                <strong>尚未选择授权</strong>
                <button type="button" disabled={grantBusy} onClick={newGrant}><Plus size={14} />新建授权</button>
              </section>
            )}
          </div>
        ) : null}

        {tab === "http" ? (
          <section className="mcp-http-view" role="tabpanel">
            <div className="mcp-http-panel">
              <header><strong>Streamable HTTP</strong><span aria-live="polite">{httpDirty ? "配置未保存" : httpConfig?.tokenAvailable ? "Token 已保存" : "未生成 Token"}</span></header>
              <div className="mcp-http-settings">
                <div className="mcp-http-field-grid">
                  <McpFieldGroup label="监听 IP:">
                    <div className="mcp-http-listen-editor">
                      <select aria-label="MCP HTTP 监听范围" value={mcpHttpListenPreset(httpSettings.listenHost)} disabled={httpControlsLocked} onChange={(event) => {
                        const listenHost = event.target.value === MCP_HTTP_CUSTOM_LISTEN_PRESET ? "" : event.target.value;
                        updateHttpListenHost(listenHost);
                        if (!listenHost) requestAnimationFrame(() => httpListenInputRef.current?.focus());
                      }}>
                        {mcpHttpListenOptions.map(([value, label]) => <option key={value} value={value}>{label}</option>)}
                      </select>
                      <input ref={httpListenInputRef} aria-label="MCP HTTP 监听 IP" value={httpSettings.listenHost} maxLength={128} spellCheck={false} disabled={httpControlsLocked} onChange={(event) => updateHttpListenHost(event.target.value)} />
                    </div>
                  </McpFieldGroup>
                  <McpField label="端口:"><input aria-label="MCP HTTP 端口" type="number" min={1} max={65_535} value={httpSettings.port || ""} disabled={httpControlsLocked} onChange={(event) => updateHttpSettings({ port: Number(event.target.value) })} /></McpField>
                  <McpField label="Client ID:"><input aria-label="MCP HTTP Client ID" list="mcp-http-client-ids" value={httpSettings.clientId} maxLength={128} spellCheck={false} disabled={httpControlsLocked} onChange={(event) => updateHttpSettings({ clientId: event.target.value })} /><datalist id="mcp-http-client-ids">{grants.filter((grant) => !grant.revokedAt).map((grant) => <option key={grant.clientId} value={grant.clientId}>{grant.name}</option>)}</datalist></McpField>
                  <McpField label="客户端地址:"><input aria-label="MCP HTTP 客户端地址" value={httpSettings.clientHost} maxLength={253} spellCheck={false} disabled={httpControlsLocked} placeholder="192.168.33.222" onChange={(event) => updateHttpSettings({ clientHost: event.target.value })} /></McpField>
                </div>
                <McpField label="Allowed Origins:"><textarea className="mcp-http-origins" aria-label="MCP HTTP Allowed Origins" value={httpOriginsText} spellCheck={false} placeholder="https://console.example.com" disabled={httpControlsLocked} onChange={(event) => { requestGateRef.current.invalidate("http-preview"); setHttpOriginsText(event.target.value); setHttpDirty(true); setHttpPreviewCurrent(false); setError(""); }} /></McpField>
                <div className="mcp-http-options">
                  <label><input type="checkbox" checked={httpSettings.allowRemote} disabled={httpControlsLocked || !httpRemoteListener} onChange={(event) => updateHttpSettings({ allowRemote: event.target.checked })} />允许非本机监听</label>
                  <label><input type="checkbox" checked={httpSettings.trusted} disabled={httpControlsLocked} onChange={(event) => updateHttpSettings({ trusted: event.target.checked })} />授权为空时允许写操作</label>
                </div>
                {httpRemoteListener ? (
                  <div className={`mcp-http-exposure ${httpSettings.allowRemote ? "allowed" : "blocked"}`} role="status">
                    {httpSettings.allowRemote ? "网络可达主机可访问此端点；Token 与 Origin 校验保持启用，仅用于可信网络或 TLS 代理后方。" : "非回环监听需要显式允许远程访问。"}
                  </div>
                ) : null}
              </div>
              <section className="mcp-cc-switch" aria-labelledby="mcp-cc-switch-title">
                <header>
                  <strong id="mcp-cc-switch-title">CC Switch JSON</strong>
                  <button type="button" title="复制 CC Switch JSON" aria-label="复制 CC Switch JSON" disabled={!ccSwitchJson || !httpPreviewCurrent} onClick={() => void copyCcSwitchJson()}><Copy size={14} /><span>{ccSwitchCopied ? "已复制" : "复制 JSON"}</span></button>
                </header>
                <div className="mcp-cc-switch-options">
                  <label><span>Server ID</span><input aria-label="CC Switch Server ID" value={ccSwitchServerId} maxLength={64} spellCheck={false} onChange={(event) => setCcSwitchServerId(event.target.value)} /></label>
                  <label><span>Bearer Token</span><input aria-label="CC Switch Bearer Token" value={httpToken} readOnly spellCheck={false} placeholder="先生成 Token" /></label>
                  <label><span>工具超时</span><input aria-label="CC Switch 工具超时秒数" type="number" min={1} max={3_600} value={ccSwitchToolTimeout || ""} onChange={(event) => setCcSwitchToolTimeout(Number(event.target.value))} /></label>
                </div>
                <textarea className={httpDirty && !httpPreviewCurrent ? "mcp-cc-switch-json stale" : "mcp-cc-switch-json"} readOnly aria-label="CC Switch MCP JSON" value={ccSwitchJson} />
              </section>
              <div className="mcp-http-row"><span>Listen</span><code>{httpConfig?.endpoint ?? "http://127.0.0.1:8787/mcp"}</code></div>
              <div className="mcp-http-row"><span>Client</span><code>{httpConfig?.clientEndpoint ?? mcpHttpClientEndpoint(httpSettings) ?? "-"}</code></div>
              <div className="mcp-http-row"><span>Token Ref</span><code>{httpConfig?.tokenRef ?? "keychain:mcp-http-token"}</code></div>
              <div className="mcp-http-row"><span>Executable</span><code>{httpConfig?.executable ?? "portmate-mcp"}</code></div>
              <div className="mcp-http-row"><span>Store</span><code>{httpConfig?.storePath ?? "portmate-store.sqlite3"}</code></div>
              <div className={`mcp-http-runtime ${httpRuntime?.phase ?? "stopped"}`} role="status" aria-live="polite">
                <span className="mcp-http-runtime-indicator" />
                <strong>{mcpHttpRuntimeLabel(httpRuntime)}</strong>
                {httpRuntime?.pid ? <code>PID {httpRuntime.pid}</code> : null}
                {httpRuntime?.startedAt ? <time>{formatDateTime(httpRuntime.startedAt)}</time> : null}
                {httpRuntime?.message ? <small>{httpRuntime.message}</small> : null}
              </div>
              {httpToken ? <div className="mcp-http-token"><span>新 Token</span><code>{httpToken}</code></div> : null}
              <textarea className={httpDirty && !httpPreviewCurrent ? "mcp-http-command stale" : "mcp-http-command"} readOnly aria-label="MCP HTTP 启动命令" value={httpConfig?.startCommand ?? ""} />
              {error ? <div className="utility-error">{error}</div> : null}
              <div className="mcp-actions">
                <button type="button" onClick={() => void saveHttpSettings()} disabled={httpBusy || httpRuntimeLocked || !httpSettingsValid || !httpDirty}><Save size={14} />保存配置</button>
                <button type="button" onClick={() => void rotateHttpToken()} disabled={httpBusy || httpRuntimeLocked || httpDirty || !httpConfig}><KeyRound size={14} />{httpConfig?.tokenAvailable ? "轮换 Token" : "生成 Token"}</button>
                <button type="button" onClick={() => void startHttpRuntime()} disabled={httpBusy || httpRuntimeBusy || !httpRuntime || httpRuntimeActive || httpDirty || !httpConfig?.tokenAvailable}><Play size={14} />启动服务</button>
                <button type="button" onClick={() => void stopHttpRuntime()} disabled={httpRuntimeBusy || !httpRuntime || httpRuntime.phase === "stopped"}><Square size={13} />停止服务</button>
                <button type="button" onClick={() => void navigator.clipboard?.writeText(httpConfig?.startCommand ?? "")} disabled={!httpConfig || !httpPreviewCurrent}><Copy size={14} />复制命令</button>
              </div>
            </div>
          </section>
        ) : null}

        {tab === "audit" ? (
          <section className="mcp-audit-view" role="tabpanel">
            <div className="mcp-audit-toolbar">
              <label className="mcp-audit-search"><Search size={14} /><input aria-label="筛选 MCP 审计" value={auditQuery} onChange={(event) => setAuditQuery(event.target.value)} placeholder="client、动作或详情" /></label>
              <select aria-label="筛选审计决策" value={auditDecision} onChange={(event) => setAuditDecision(event.target.value)}><option value="">全部决策</option>{decisionOptions.map((decision) => <option key={decision} value={decision}>{decision}</option>)}</select>
              <select aria-label="筛选审计会话" value={auditSessionId} onChange={(event) => setAuditSessionId(event.target.value)}><option value="">全部会话</option><option value={MCP_AUDIT_GLOBAL_SESSION}>全局</option>{auditSessionOptions.map((id) => <option key={id} value={id}>{sessionNames.get(id) ?? id}</option>)}</select>
              <select aria-label="筛选审计权限" value={auditScope} onChange={(event) => setAuditScope(event.target.value as "" | McpScope)}><option value="">全部 scope</option>{allMcpScopes.map((scope) => <option key={scope} value={scope}>{scope}</option>)}</select>
              <span className="mcp-audit-count">{filteredAudit.length} / {audit.length}</span>
              <button type="button" className="icon-button" title="刷新审计" aria-label="刷新 MCP 审计" disabled={auditBusy || !isBackendAvailable()} onClick={() => void refreshAudit()}><RefreshCw size={14} /></button>
              <button type="button" className="icon-button" title="导出筛选结果" aria-label="导出 MCP 审计" disabled={auditBusy || !filteredAudit.length || !isBackendAvailable()} onClick={() => void exportAudit()}><Download size={14} /></button>
            </div>
            {auditExport ? <div className="mcp-audit-export"><span>已导出 {auditExport.records} 条 · SHA-256 {auditExport.sha256.slice(0, 12)}...</span><button type="button" title="复制导出信息" aria-label="复制 MCP 审计导出信息" onClick={() => void navigator.clipboard?.writeText(`${auditExport.path}\n${auditExport.checksumPath}\nSHA-256 ${auditExport.sha256}`).catch(() => {})}><Copy size={14} /></button></div> : null}
            {error ? <div className="utility-error">{error}</div> : null}
            <div className="mcp-audit-workspace">
              <div className="mcp-audit-list" role="listbox" aria-label="MCP 审计记录">
                {filteredAudit.map((record) => (
                  <button key={record.id} type="button" role="option" aria-selected={record.id === selectedAudit?.id} className={record.id === selectedAudit?.id ? "active" : ""} onClick={() => setSelectedAuditId(record.id)}>
                    <span><strong>{record.action}</strong><time>{formatDateTime(record.ts)}</time></span>
                    <span><code>{record.actor}</code><em className={`decision-${record.decision}`}>{record.decision}</em></span>
                    <small>{record.sessionId ? sessionNames.get(record.sessionId) ?? record.sessionId : "全局"} · {record.details.scope ?? "scope unknown"}</small>
                  </button>
                ))}
                {!filteredAudit.length ? <div className="empty-pane top">没有匹配的审计记录</div> : null}
              </div>
              <div className="mcp-audit-inspector">
                {selectedAudit ? <AuditInspector record={selectedAudit} sessionName={selectedAudit.sessionId ? sessionNames.get(selectedAudit.sessionId) : undefined} /> : <div className="empty-pane top">选择一条审计记录</div>}
              </div>
            </div>
          </section>
        ) : null}
      </section>
    </div>
  );
}

function AuditInspector({ record, sessionName }: { record: AuditRecord; sessionName?: string }) {
  return (
    <>
      <header><strong>{record.action}</strong><span>{record.decision}</span></header>
      <dl>
        <div><dt>时间</dt><dd>{formatDateTime(record.ts)}</dd></div>
        <div><dt>Client</dt><dd><code>{record.actor}</code></dd></div>
        <div><dt>会话</dt><dd>{record.sessionId ? <><span>{sessionName ?? record.sessionId}</span><code>{record.sessionId}</code></> : "全局"}</dd></div>
        <div><dt>记录 ID</dt><dd><code>{record.id}</code></dd></div>
        {Object.entries(record.details).sort(([left], [right]) => left.localeCompare(right)).map(([key, value]) => <div key={key}><dt>{key}</dt><dd><code>{value}</code></dd></div>)}
      </dl>
    </>
  );
}

function McpField({ label, children }: { label: string; children: ReactNode }) {
  return <label className="dialog-field"><span>{label}</span>{children}</label>;
}

function McpFieldGroup({ label, children }: { label: string; children: ReactNode }) {
  return <div className="dialog-field"><span>{label}</span>{children}</div>;
}

function mcpHttpRuntimeLabel(status: McpHttpRuntimeStatus | null) {
  switch (status?.phase) {
    case "starting": return "正在启动";
    case "running": return "运行中";
    case "failed": return "启动失败";
    case "stopped": return "未运行";
    default: return "读取状态";
  }
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
