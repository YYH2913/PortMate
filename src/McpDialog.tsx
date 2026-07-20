import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import { Copy, Download, Plus, RefreshCw, Search, X } from "lucide-react";
import { invokeBackend, isBackendAvailable } from "./api";
import { filterMcpAudit, MCP_AUDIT_GLOBAL_SESSION, mcpAuditDecisionOptions } from "./mcp-audit-state";
import type { AuditRecord, ExportMcpAuditResult, McpGrant, McpHttpConfig, McpHttpTokenResponse, McpScope, SessionSummary } from "./types";

const allMcpScopes: McpScope[] = ["read-sessions", "read-logs", "write-input", "transfer", "tunnel", "manage-sessions"];
type McpDialogTab = "grants" | "http" | "audit";

export default function McpDialog({
  grants,
  audit,
  sessions,
  onClose,
  onGrantChange,
  onAuditChange,
}: {
  grants: McpGrant[];
  audit: AuditRecord[];
  sessions: SessionSummary[];
  onClose: () => void;
  onGrantChange: (grants: McpGrant[]) => void;
  onAuditChange: (audit: AuditRecord[]) => void;
}) {
  const [tab, setTab] = useState<McpDialogTab>("grants");
  const [draft, setDraft] = useState<McpGrant>(() => grants[0] ?? createMcpGrant());
  const [editingClientId, setEditingClientId] = useState<string | null>(() => grants[0]?.clientId ?? null);
  const [error, setError] = useState("");
  const [httpConfig, setHttpConfig] = useState<McpHttpConfig | null>(null);
  const [httpToken, setHttpToken] = useState("");
  const [httpBusy, setHttpBusy] = useState(false);
  const [auditBusy, setAuditBusy] = useState(false);
  const [auditQuery, setAuditQuery] = useState("");
  const [auditDecision, setAuditDecision] = useState("");
  const [auditSessionId, setAuditSessionId] = useState("");
  const [auditScope, setAuditScope] = useState<"" | McpScope>("");
  const [selectedAuditId, setSelectedAuditId] = useState("");
  const [auditExport, setAuditExport] = useState<ExportMcpAuditResult | null>(null);

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

  useEffect(() => {
    if (tab !== "http" || httpConfig || !isBackendAvailable()) return;
    void loadHttpConfig();
  }, [httpConfig, tab]);

  async function loadHttpConfig() {
    try {
      setHttpConfig(await invokeBackend<McpHttpConfig>("mcp_http_config", {}));
    } catch (nextError) {
      setError(formatError(nextError));
    }
  }

  async function rotateHttpToken() {
    setError("");
    setHttpBusy(true);
    try {
      const response = await invokeBackend<McpHttpTokenResponse>("rotate_mcp_http_token", {});
      setHttpConfig(response.config);
      setHttpToken(response.token);
    } catch (nextError) {
      setError(formatError(nextError));
    } finally {
      setHttpBusy(false);
    }
  }

  async function saveGrant() {
    setError("");
    try {
      const saved = await invokeBackend<McpGrant[]>("save_mcp_grant", { grant: draft });
      onGrantChange(saved);
      const selected = saved.find((grant) => grant.clientId === draft.clientId);
      if (selected) {
        setDraft(selected);
        setEditingClientId(selected.clientId);
      }
    } catch (nextError) {
      setError(formatError(nextError));
    }
  }

  async function revokeGrant(clientId: string) {
    setError("");
    try {
      const saved = await invokeBackend<McpGrant[]>("revoke_mcp_grant", { clientId });
      onGrantChange(saved);
      setDraft(saved[0] ?? createMcpGrant());
      setEditingClientId(saved[0]?.clientId ?? null);
    } catch (nextError) {
      setError(formatError(nextError));
    }
  }

  async function refreshAudit() {
    setError("");
    setAuditBusy(true);
    try {
      onAuditChange(await invokeBackend<AuditRecord[]>("list_mcp_audit", {}));
    } catch (nextError) {
      setError(formatError(nextError));
    } finally {
      setAuditBusy(false);
    }
  }

  async function exportAudit() {
    setError("");
    setAuditBusy(true);
    setAuditExport(null);
    try {
      setAuditExport(await invokeBackend<ExportMcpAuditResult>("export_mcp_audit", {
        request: { recordIds: filteredAudit.map((record) => record.id) },
      }));
    } catch (nextError) {
      setError(formatError(nextError));
    } finally {
      setAuditBusy(false);
    }
  }

  function selectGrant(grant: McpGrant) {
    setDraft(grant);
    setEditingClientId(grant.clientId);
    setError("");
  }

  function newGrant() {
    setDraft(createMcpGrant());
    setEditingClientId(null);
    setError("");
  }

  function toggleScope(scope: McpScope) {
    setDraft((current) => ({
      ...current,
      scopes: current.scopes.includes(scope) ? current.scopes.filter((item) => item !== scope) : [...current.scopes, scope],
    }));
  }

  function toggleSession(sessionId: string) {
    setDraft((current) => ({
      ...current,
      allowedSessions: current.allowedSessions.includes(sessionId) ? current.allowedSessions.filter((item) => item !== sessionId) : [...current.allowedSessions, sessionId],
    }));
  }

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section className="wind-dialog mcp-dialog" data-tab={tab} aria-label="MCP Bridge">
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>MCP Bridge</strong>
          <button type="button" title="关闭" aria-label="关闭 MCP Bridge" onClick={onClose}><X size={20} /></button>
        </header>
        <nav className="mcp-tabs" role="tablist" aria-label="MCP Bridge 视图">
          {([['grants', '授权'], ['http', 'HTTP'], ['audit', '审计']] as const).map(([id, label]) => (
            <button key={id} type="button" role="tab" aria-selected={tab === id} className={tab === id ? "active" : ""} onClick={() => { setTab(id); setError(""); }}>{label}</button>
          ))}
        </nav>

        {tab === "grants" ? (
          <div className="mcp-content" role="tabpanel">
            <aside className="mcp-grants">
              <button type="button" className="mcp-new" onClick={newGrant}><Plus size={14} />新建授权</button>
              {grants.map((grant) => (
                <button key={grant.clientId} type="button" className={grant.clientId === editingClientId ? "active" : ""} onClick={() => selectGrant(grant)}>
                  <strong>{grant.name || grant.clientId}</strong>
                  <span>{grant.scopes.join(", ") || "read-only"}</span>
                </button>
              ))}
              {!grants.length ? <div className="empty-pane top">没有授权规则</div> : null}
            </aside>
            <section className="mcp-editor">
              <McpField label="Client ID:"><input value={draft.clientId} readOnly={editingClientId !== null} onChange={(event) => setDraft({ ...draft, clientId: event.target.value })} /></McpField>
              <McpField label="名称:"><input value={draft.name} onChange={(event) => setDraft({ ...draft, name: event.target.value })} /></McpField>
              <McpField label="写操作:"><span className="mcp-confirm-write"><input type="checkbox" aria-label="写操作每次确认" checked={Boolean(draft.confirmWrites)} onChange={(event) => setDraft({ ...draft, confirmWrites: event.target.checked })} />每次确认</span></McpField>
              <fieldset className="mcp-check-grid">
                <legend>权限范围</legend>
                {allMcpScopes.map((scope) => <label key={scope}><input type="checkbox" checked={draft.scopes.includes(scope)} onChange={() => toggleScope(scope)} />{scope}</label>)}
              </fieldset>
              <fieldset className="mcp-session-list">
                <legend>允许会话 · {draft.allowedSessions.length ? `${draft.allowedSessions.length} 个` : "全部"}</legend>
                {sessions.map((session) => <label key={session.profile.id}><input type="checkbox" checked={draft.allowedSessions.includes(session.profile.id)} onChange={() => toggleSession(session.profile.id)} />{session.profile.name}</label>)}
              </fieldset>
              {error ? <div className="utility-error">{error}</div> : null}
              <div className="mcp-actions">
                <button type="button" onClick={() => void saveGrant()}>保存</button>
                <button type="button" onClick={() => void revokeGrant(draft.clientId)} disabled={!editingClientId}>撤销</button>
              </div>
            </section>
          </div>
        ) : null}

        {tab === "http" ? (
          <section className="mcp-http-view" role="tabpanel">
            <div className="mcp-http-panel">
              <header><strong>Streamable HTTP</strong><span>{httpConfig?.tokenAvailable ? "token 已保存" : "未生成 token"}</span></header>
              <div className="mcp-http-row"><span>Endpoint</span><code>{httpConfig?.endpoint ?? "http://127.0.0.1:8787/mcp"}</code></div>
              <div className="mcp-http-row"><span>Origin</span><code>{httpConfig?.defaultOrigin ?? "http://127.0.0.1:8787"}</code></div>
              <div className="mcp-http-row"><span>Token Ref</span><code>{httpConfig?.tokenRef ?? "keychain:mcp-http-token"}</code></div>
              <div className="mcp-http-row"><span>Executable</span><code>{httpConfig?.executable ?? "portmate-mcp"}</code></div>
              <div className="mcp-http-row"><span>Store</span><code>{httpConfig?.storePath ?? "portmate-store.sqlite3"}</code></div>
              {httpToken ? <div className="mcp-http-token"><span>新 Token</span><code>{httpToken}</code></div> : null}
              <textarea readOnly aria-label="MCP HTTP 启动命令" value={httpConfig?.startCommand ?? "portmate-mcp --http"} />
              {error ? <div className="utility-error">{error}</div> : null}
              <div className="mcp-actions">
                <button type="button" onClick={() => void rotateHttpToken()} disabled={httpBusy}>{httpConfig?.tokenAvailable ? "轮换 Token" : "生成 Token"}</button>
                <button type="button" onClick={() => void navigator.clipboard?.writeText(httpConfig?.startCommand ?? "")} disabled={!httpConfig}>复制启动命令</button>
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

function createMcpGrant(): McpGrant {
  return { clientId: "portmate-local", name: "Local MCP Client", scopes: ["read-sessions", "read-logs"], allowedSessions: [], confirmWrites: true, expiresAt: null, revokedAt: null };
}

function formatDateTime(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function formatError(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
