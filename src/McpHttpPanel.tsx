import { useRef, useState } from "react";
import { Copy, KeyRound, Play, RefreshCw, Save, Square } from "lucide-react";
import { mcpSessionAccessMode } from "./mcp-grant-state";
import { isNonLoopbackMcpHost, MCP_HTTP_CUSTOM_LISTEN_PRESET, mcpHttpClientEndpoint, mcpHttpListenPreset } from "./mcp-http-state";
import { mcpHttpRuntimeLabel } from "./mcp-http-workflow";
import type { McpHttpController } from "./use-mcp-http-controller";

const listenOptions = [
  ["127.0.0.1", "仅本机 · IPv4"], ["0.0.0.0", "所有网卡 · IPv4"],
  ["::1", "仅本机 · IPv6"], ["::", "所有网卡 · IPv6"],
  [MCP_HTTP_CUSTOM_LISTEN_PRESET, "指定 IP"],
] as const;

export default function McpHttpPanel({ http, grantBusy, onManageGrants }: {
  http: McpHttpController;
  grantBusy: boolean;
  onManageGrants: () => void;
}) {
  const listenInput = useRef<HTMLInputElement>(null);
  const [reveal, setReveal] = useState(false);
  const { settings, savedConfig, selectedGrant, savedGrant, runtime, running, dirty } = http;
  const locked = http.locked || grantBusy;
  const actionBusy = http.busy || http.loading || http.copying || grantBusy;
  const startLabel = dirty ? "保存并启动" : !http.token ? "生成 Token 并启动" : "启动服务";
  const mode = selectedGrant ? mcpSessionAccessMode(selectedGrant) : null;
  const currentBinding = savedGrant?.name || savedGrant?.clientId;

  function updateListenHost(listenHost: string) {
    http.update({ listenHost, ...(!isNonLoopbackMcpHost(listenHost) ? { allowRemote: false } : {}) });
  }

  return (
    <section className="mcp-http-view" role="tabpanel" aria-label="HTTP 接入">
      <div className="mcp-http-panel">
        <header className="mcp-service-toolbar">
          <div className={`mcp-http-runtime ${runtime?.phase ?? "stopped"}`} role="status" aria-live="polite">
            <span className="mcp-http-runtime-indicator" />
            <strong>{mcpHttpRuntimeLabel(runtime)}</strong>
            {runtime?.pid ? <code>PID {runtime.pid}</code> : null}
            <small>{currentBinding ? `当前授权：${currentBinding}` : "尚未绑定有效授权"}</small>
          </div>
          <div className="mcp-actions">
            <button type="button" className="icon-button" aria-label="刷新 HTTP 状态" title="刷新状态" disabled={actionBusy} onClick={http.refresh}><RefreshCw size={14} /></button>
            {running ? (
              <button type="button" disabled={actionBusy} onClick={() => void http.mutate("stop")}><Square size={13} />停止服务</button>
            ) : (
              <button type="button" className="primary" disabled={locked || !http.settingsValid} onClick={() => void http.mutate("start")}><Play size={14} />{http.busy ? "处理中…" : startLabel}</button>
            )}
          </div>
        </header>
        {runtime?.message ? <div className="utility-error" role="alert">{runtime.message}</div> : null}
        {http.error ? <div className="utility-error" role="alert">{http.error}</div> : null}
        {http.notice ? <p className="mcp-inline-notice" role="status">{http.notice}</p> : null}

        <section className="mcp-http-identity" aria-labelledby="mcp-http-identity-title">
          <header className="mcp-section-heading">
            <div><strong id="mcp-http-identity-title">授权身份</strong><span>一个 HTTP 端点使用一个授权身份。持有同一 Token 的连接共享其权限。</span></div>
            <button type="button" className="mcp-inline-link" onClick={onManageGrants}>管理授权 →</button>
          </header>
          <div className="mcp-binding-selector">
            <label><span>授权客户端</span>
              <select aria-label="MCP HTTP Client ID" value={selectedGrant?.clientId ?? ""} disabled={locked || !http.activeGrants.length}
                onChange={(event) => http.update({ clientId: event.target.value })}>
                <option value="" disabled>请选择已保存的有效授权</option>
                {http.activeGrants.map(grant => <option key={grant.clientId} value={grant.clientId}>{grant.name || grant.clientId} · {grant.clientId}</option>)}
              </select>
            </label>
            {selectedGrant ? <span className="mcp-binding-summary">{selectedGrant.scopes.length} 项权限 · {mode === "all" ? "全部会话" : mode === "none" ? "不授权会话" : `${selectedGrant.allowedSessions.length} 个会话`}</span>
              : <span className="mcp-binding-summary">{http.activeGrants.length ? "请从列表选择授权" : "先创建客户端授权"}</span>}
          </div>
          {savedConfig && !savedGrant ? <p className="mcp-inline-warning" role="status">已保存的身份 <code>{savedConfig.clientId}</code> 没有有效授权；不会自动切换到其他客户端。</p> : null}
          {http.identityChanged ? <p className="mcp-inline-warning">切换身份后旧 Token 将失效，已有连接需更新接入配置。</p> : null}
          {running ? <p className="mcp-help">服务运行时锁定绑定和网络配置。如需修改，请先停止服务；权限仍在授权页管理。</p> : null}
        </section>

        <div className="mcp-http-columns">
          <section className="mcp-http-network" aria-labelledby="mcp-http-network-title">
            <header className="mcp-section-heading"><div><strong id="mcp-http-network-title">网络配置</strong><span>默认仅本机可访问。远程监听需要明确允许。</span></div></header>
            <div className="mcp-network-fields">
              <label><span>监听范围</span><select aria-label="MCP HTTP 监听范围" value={mcpHttpListenPreset(settings.listenHost)} disabled={locked} onChange={(event) => {
                const host = event.target.value === MCP_HTTP_CUSTOM_LISTEN_PRESET ? "" : event.target.value;
                updateListenHost(host);
                if (!host) requestAnimationFrame(() => listenInput.current?.focus());
              }}>{listenOptions.map(([value, label]) => <option key={value} value={value}>{label}</option>)}</select></label>
              <div className="mcp-address-fields">
                <label><span>监听 IP</span><input ref={listenInput} aria-label="MCP HTTP 监听 IP" value={settings.listenHost} disabled={locked} maxLength={128} spellCheck={false} onChange={event => updateListenHost(event.target.value)} /></label>
                <label><span>端口</span><input type="number" aria-label="MCP HTTP 端口" min={1} max={65_535} value={settings.port || ""} disabled={locked} onChange={event => http.update({ port: Number(event.target.value) })} /></label>
              </div>
              <label><span>接入地址（PortMate 主机）</span><input aria-label="MCP HTTP 客户端地址" value={settings.clientHost} maxLength={253} spellCheck={false} disabled={locked} onChange={event => http.update({ clientHost: event.target.value })} /></label>
              <p className="mcp-help">客户端用此 IP 或域名连接 PortMate，不是客户端自身的地址。</p>
              <label className="mcp-remote-consent"><input type="checkbox" checked={settings.allowRemote} disabled={locked || !http.remote} onChange={event => http.update({ allowRemote: event.target.checked })} />允许非本机监听</label>
              {http.remote ? <p className="mcp-inline-warning" role="status">{settings.allowRemote ? "只用于可信网络或 TLS 代理后方，HTTP 本身不加密。" : "请确认允许非本机监听后再保存或启动。"}</p> : null}
              <details className="mcp-advanced">
                <summary>浏览器来源限制 · Allowed Origins</summary>
                <label><span>允许的 Origin（每行一个）</span><textarea aria-label="MCP HTTP Allowed Origins" value={http.originsText} disabled={locked} spellCheck={false} onChange={event => http.updateOrigins(event.target.value)} /></label>
                <p className="mcp-help">限制携带 Origin 的浏览器请求，不替代 Token 校验或防火墙。</p>
              </details>
            </div>
            <div className="mcp-actions mcp-network-actions">
              <span>{dirty ? "有未保存更改" : "配置已保存"}</span>
              {dirty ? <button type="button" disabled={locked} onClick={http.resetDraft}>放弃更改</button> : null}
              <button type="button" disabled={locked || !http.settingsValid || !dirty} onClick={() => void http.mutate("save")}><Save size={14} />保存配置</button>
            </div>
          </section>

          <section className="mcp-cc-switch" aria-labelledby="mcp-cc-switch-title">
            <header className="mcp-section-heading"><div><strong id="mcp-cc-switch-title">客户端接入</strong><small>{dirty ? "保存配置后可复制完整 JSON" : "复制到 CC Switch 或支持 HTTP 的 MCP 客户端"}</small></div></header>
            <div className="mcp-http-row"><span>接入 URL</span><code>{savedConfig?.clientEndpoint ?? mcpHttpClientEndpoint(settings) ?? "—"}</code></div>
            <div className="mcp-connection-actions mcp-actions">
              <button type="button" className="primary" aria-label="复制 CC Switch JSON" disabled={!http.json || actionBusy} onClick={() => void http.copy("json")}><Copy size={14} />{http.copied ? "已复制" : "复制接入 JSON"}</button>
              <button type="button" disabled={locked || dirty || !savedGrant} onClick={() => void http.mutate("rotate")}><KeyRound size={14} />{http.token ? "轮换 Token" : "生成 Token"}</button>
            </div>
            <p className="mcp-help">{dirty ? "预览和复制已暂停，避免使用旧 Token。" : !http.token ? "点击顶部启动按钮可自动生成 Token 并启动服务。" : "接入 JSON 包含 Token，请按密码保管。复制不会更改权限或服务配置。"}</p>
            <div className="mcp-cc-switch-options">
              <label><span>Server ID</span><input aria-label="CC Switch Server ID" value={http.serverId} maxLength={64} spellCheck={false} onChange={event => http.setServerId(event.target.value)} /></label>
              <label><span>工具超时（秒）</span><input aria-label="CC Switch 工具超时秒数" type="number" min={1} max={3_600} value={http.toolTimeout || ""} onChange={event => http.setToolTimeout(Number(event.target.value))} /></label>
            </div>
            <details className="mcp-advanced mcp-access-preview" onToggle={event => setReveal(event.currentTarget.open)}>
              <summary>显示接入配置（含 Token）</summary>
              <label><span>Bearer Token</span><input aria-label="CC Switch Bearer Token" readOnly value={reveal && !dirty && savedGrant && !actionBusy ? http.token : ""} /></label>
              <textarea aria-label="CC Switch MCP JSON" readOnly value={reveal && !actionBusy ? http.json : ""} />
            </details>
          </section>
        </div>
        <details className="mcp-advanced mcp-process-details">
          <summary>进程信息与手动启动</summary>
          <div className="mcp-http-row"><span>监听 URL</span><code>{savedConfig?.endpoint ?? "—"}</code></div>
          <div className="mcp-http-row"><span>Executable</span><code>{savedConfig?.executable ?? "—"}</code></div>
          <div className="mcp-http-row"><span>Store</span><code>{savedConfig?.storePath ?? "—"}</code></div>
          <div className="mcp-http-row"><span>Token Ref</span><code>{savedConfig?.tokenRef ?? "—"}</code></div>
          <textarea className="mcp-http-command" aria-label="MCP HTTP 启动命令" readOnly value={!dirty && savedGrant ? savedConfig?.startCommand ?? "" : ""} />
          <div className="mcp-actions"><button type="button" disabled={actionBusy || dirty || !savedGrant} onClick={() => void http.copy("command")}><Copy size={14} />{http.commandCopied ? "已复制" : "复制命令"}</button></div>
        </details>
      </div>
    </section>
  );
}
