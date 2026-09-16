import { t, useLocale, localizeDiagnostic } from "./i18n";
import { useRef, useState } from "react";
import { Copy, KeyRound, Play, RefreshCw, Save, Square } from "lucide-react";
import { mcpSessionAccessMode } from "./mcp-grant-state";
import { isNonLoopbackMcpHost, MCP_HTTP_CUSTOM_LISTEN_PRESET, mcpHttpClientEndpoint, mcpHttpListenPreset } from "./mcp-http-state";
import { mcpHttpRuntimeLabel } from "./mcp-http-workflow";
import type { McpHttpController } from "./use-mcp-http-controller";

const listenOptions = [
  ["127.0.0.1", "local-only-ipv4"], ["0.0.0.0", "all-interfaces-ipv4"],
  ["::1", "local-only-ipv6"], ["::", "all-interfaces-ipv6"],
  [MCP_HTTP_CUSTOM_LISTEN_PRESET, "specific-ip"],
] as const;

export default function McpHttpPanel({ http, grantBusy, onManageGrants }: {
  http: McpHttpController;
  grantBusy: boolean;
  onManageGrants: () => void;
}) {
  useLocale();
  const listenInput = useRef<HTMLInputElement>(null);
  const [reveal, setReveal] = useState(false);
  const { settings, savedConfig, selectedGrant, savedGrant, runtime, running, dirty } = http;
  const locked = http.locked || grantBusy;
  const actionBusy = http.busy || http.loading || http.copying || grantBusy;
  const startLabel = dirty ? t("save-and-start") : !http.token ? t("generate-token-and-start") : t("start-service");
  const mode = selectedGrant ? mcpSessionAccessMode(selectedGrant) : null;
  const currentBinding = savedGrant?.name || savedGrant?.clientId;

  function updateListenHost(listenHost: string) {
    http.update({ listenHost, ...(!isNonLoopbackMcpHost(listenHost) ? { allowRemote: false } : {}) });
  }

  return (
    <section className="mcp-http-view" role="tabpanel" aria-label={t("http-access")}>
      <div className="mcp-http-panel">
        <header className="mcp-service-toolbar">
          <div className={`mcp-http-runtime ${runtime?.phase ?? "stopped"}`} role="status" aria-live="polite">
            <span className="mcp-http-runtime-indicator" />
            <strong>{mcpHttpRuntimeLabel(runtime)}</strong>
            {runtime?.pid ? <code>PID {runtime.pid}</code> : null}
            <small>{currentBinding ? t("current-grant", [currentBinding]) : t("no-active-grant-bound")}</small>
          </div>
          <div className="mcp-actions">
            <button type="button" className="icon-button" aria-label={t("refresh-http-status")} title={t("refresh-status")} disabled={actionBusy} onClick={http.refresh}><RefreshCw size={14} /></button>
            {running ? (
              <button type="button" disabled={actionBusy} onClick={() => void http.mutate("stop")}><Square size={13} />{t("stop-service")}</button>
            ) : (
              <button type="button" className="primary" disabled={locked || !http.settingsValid} onClick={() => void http.mutate("start")}><Play size={14} />{http.busy ? t("processing") : startLabel}</button>
            )}
          </div>
        </header>
        {runtime?.message ? <div className="utility-error" role="alert">{runtime.message}</div> : null}
        {http.error ? <div className="utility-error" role="alert">{localizeDiagnostic(http.error)}</div> : null}
        {http.notice ? <p className="mcp-inline-notice" role="status">{http.notice}</p> : null}

        <section className="mcp-http-identity" aria-labelledby="mcp-http-identity-title">
          <header className="mcp-section-heading">
            <div><strong id="mcp-http-identity-title">{t("authorization-identity")}</strong><span>{t("one-http-endpoint-uses-one-authorization-identity-connections-with")}</span></div>
            <button type="button" className="mcp-inline-link" onClick={onManageGrants}>{t("manage-grants")}</button>
          </header>
          <div className="mcp-binding-selector">
            <label><span>{t("authorized-client")}</span>
              <select aria-label={t("ui-mcp-http-client-id")} value={selectedGrant?.clientId ?? ""} disabled={locked || !http.activeGrants.length}
                onChange={(event) => http.update({ clientId: event.target.value })}>
                <option value="" disabled>{t("select-a-saved-active-grant")}</option>
                {http.activeGrants.map(grant => <option key={grant.clientId} value={grant.clientId}>{grant.name || grant.clientId} · {grant.clientId}</option>)}
              </select>
            </label>
            {selectedGrant ? <span className="mcp-binding-summary">{selectedGrant.scopes.length}{t("permissions-3")}{mode === "all" ? t("all-sessions") : mode === "none" ? t("no-session-access") : t("sessions", [selectedGrant.allowedSessions.length])}</span>
              : <span className="mcp-binding-summary">{http.activeGrants.length ? t("select-a-grant-from-the-list") : t("create-a-client-grant-first")}</span>}
          </div>
          {savedConfig && !savedGrant ? <p className="mcp-inline-warning" role="status">{t("the-saved-identity")}<code>{savedConfig.clientId}</code>{t("has-no-active-grant-another-client-will-not-be")}</p> : null}
          {http.identityChanged ? <p className="mcp-inline-warning">{t("changing-identity-invalidates-the-old-token-existing-connections-must")}</p> : null}
          {running ? <p className="mcp-help">{t("binding-and-network-settings-are-locked-while-the-service")}</p> : null}
        </section>

        <div className="mcp-http-columns">
          <section className="mcp-http-network" aria-labelledby="mcp-http-network-title">
            <header className="mcp-section-heading"><div><strong id="mcp-http-network-title">{t("network-settings")}</strong><span>{t("local-access-only-by-default-remote-listening-requires-explicit")}</span></div></header>
            <div className="mcp-network-fields">
              <label><span>{t("listen-scope")}</span><select aria-label={t("mcp-http-listen-scope")} value={mcpHttpListenPreset(settings.listenHost)} disabled={locked} onChange={(event) => {
                const host = event.target.value === MCP_HTTP_CUSTOM_LISTEN_PRESET ? "" : event.target.value;
                updateListenHost(host);
                if (!host) requestAnimationFrame(() => listenInput.current?.focus());
              }}>{listenOptions.map(([value, label]) => <option key={value} value={value}>{t(label)}</option>)}</select></label>
              <div className="mcp-address-fields">
                <label><span>{t("listen-ip")}</span><input ref={listenInput} aria-label={t("mcp-http-listen-ip")} value={settings.listenHost} disabled={locked} maxLength={128} spellCheck={false} onChange={event => updateListenHost(event.target.value)} /></label>
                <label><span>{t("port")}</span><input type="number" aria-label={t("mcp-http-port")} min={1} max={65_535} value={settings.port || ""} disabled={locked} onChange={event => http.update({ port: Number(event.target.value) })} /></label>
              </div>
              <label><span>{t("access-address-portmate-host")}</span><input aria-label={t("mcp-http-client-address")} value={settings.clientHost} maxLength={253} spellCheck={false} disabled={locked} onChange={event => http.update({ clientHost: event.target.value })} /></label>
              <p className="mcp-help">{t("clients-use-this-ip-or-domain-to-reach-portmate")}</p>
              <label className="mcp-remote-consent"><input type="checkbox" checked={settings.allowRemote} disabled={locked || !http.remote} onChange={event => http.update({ allowRemote: event.target.checked })} />{t("allow-non-local-listening")}</label>
              {http.remote ? <p className="mcp-inline-warning" role="status">{settings.allowRemote ? t("use-only-on-trusted-networks-or-behind-a-tls") : t("allow-non-local-listening-before-saving-or-starting")}</p> : null}
              <details className="mcp-advanced">
                <summary>{t("browser-origin-restrictions-allowed-origins")}</summary>
                <label><span>{t("allowed-origins-one-per-line")}</span><textarea aria-label={t("ui-mcp-http-allowed-origins")} value={http.originsText} disabled={locked} spellCheck={false} onChange={event => http.updateOrigins(event.target.value)} /></label>
                <p className="mcp-help">{t("restricts-browser-requests-containing-origin-it-does-not-replace")}</p>
              </details>
            </div>
            <div className="mcp-actions mcp-network-actions">
              <span>{dirty ? t("unsaved-changes") : t("settings-saved")}</span>
              {dirty ? <button type="button" disabled={locked} onClick={http.resetDraft}>{t("discard-changes")}</button> : null}
              <button type="button" disabled={locked || !http.settingsValid || !dirty} onClick={() => void http.mutate("save")}><Save size={14} />{t("save-settings")}</button>
            </div>
          </section>

          <section className="mcp-cc-switch" aria-labelledby="mcp-cc-switch-title">
            <header className="mcp-section-heading"><div><strong id="mcp-cc-switch-title">{t("client-access")}</strong><small>{dirty ? t("save-settings-to-copy-the-complete-json") : t("copy-to-cc-switch-or-an-http-compatible-mcp")}</small></div></header>
            <div className="mcp-http-row"><span>{t("access-url")}</span><code>{savedConfig?.clientEndpoint ?? mcpHttpClientEndpoint(settings) ?? "—"}</code></div>
            <div className="mcp-connection-actions mcp-actions">
              <button type="button" className="primary" aria-label={t("copy-cc-switch-json")} disabled={!http.json || actionBusy} onClick={() => void http.copy("json")}><Copy size={14} />{http.copied ? t("copied") : t("copy-access-json")}</button>
              <button type="button" disabled={locked || dirty || !savedGrant} onClick={() => void http.mutate("rotate")}><KeyRound size={14} />{http.token ? t("rotate-token") : t("generate-token")}</button>
            </div>
            <p className="mcp-help">{dirty ? t("preview-and-copying-are-paused-to-avoid-using-the") : !http.token ? t("the-start-button-above-automatically-generates-a-token-and") : t("access-json-contains-a-token-treat-it-as-a")}</p>
            <div className="mcp-cc-switch-options">
              <label><span>{t("ui-server-id")}</span><input aria-label={t("ui-cc-switch-server-id")} value={http.serverId} maxLength={64} spellCheck={false} onChange={event => http.setServerId(event.target.value)} /></label>
              <label><span>{t("tool-timeout-seconds")}</span><input aria-label={t("cc-switch-tool-timeout-in-seconds")} type="number" min={1} max={3_600} value={http.toolTimeout || ""} onChange={event => http.setToolTimeout(Number(event.target.value))} /></label>
            </div>
            <details className="mcp-advanced mcp-access-preview" onToggle={event => setReveal(event.currentTarget.open)}>
              <summary>{t("show-access-configuration-contains-token")}</summary>
              <label><span>{t("ui-bearer-token")}</span><input aria-label={t("ui-cc-switch-bearer-token")} readOnly value={reveal && !dirty && savedGrant && !actionBusy ? http.token : ""} /></label>
              <textarea aria-label={t("ui-cc-switch-mcp-json")} readOnly value={reveal && !actionBusy ? http.json : ""} />
            </details>
          </section>
        </div>
        <details className="mcp-advanced mcp-process-details">
          <summary>{t("process-details-and-manual-startup")}</summary>
          <div className="mcp-http-row"><span>{t("listen-url")}</span><code>{savedConfig?.endpoint ?? "—"}</code></div>
          <div className="mcp-http-row"><span>{t("ui-executable")}</span><code>{savedConfig?.executable ?? "—"}</code></div>
          <div className="mcp-http-row"><span>{t("ui-store")}</span><code>{savedConfig?.storePath ?? "—"}</code></div>
          <div className="mcp-http-row"><span>{t("ui-token-ref")}</span><code>{savedConfig?.tokenRef ?? "—"}</code></div>
          <textarea className="mcp-http-command" aria-label={t("mcp-http-startup-command")} readOnly value={!dirty && savedGrant ? savedConfig?.startCommand ?? "" : ""} />
          <div className="mcp-actions"><button type="button" disabled={actionBusy || dirty || !savedGrant} onClick={() => void http.copy("command")}><Copy size={14} />{http.commandCopied ? t("copied") : t("copy-command")}</button></div>
        </details>
      </div>
    </section>
  );
}
