import { t } from "./i18n";
import { useEffect, useRef, useState } from "react";
import { invokeBackend, isBackendAvailable } from "./api";
import { KeyedRequestGate } from "./keyed-request-gate";
import { mcpGrantIsActive } from "./mcp-grant-state";
import { ccSwitchServerIdForGrant, CC_SWITCH_DEFAULT_TOOL_TIMEOUT_SECONDS, defaultMcpHttpSettings, formatCcSwitchMcpJson, formatMcpHttpOrigins, isNonLoopbackMcpHost, mcpHttpClientEndpoint, mcpHttpSettingsFromConfig, parseMcpHttpOrigins } from "./mcp-http-state";
import { prepareMcpHttpAccess } from "./mcp-http-workflow";
import type { McpGrant, McpHttpAccessResponse, McpHttpConfig, McpHttpConfigRequest, McpHttpRuntimeStatus, McpHttpTokenResponse } from "./types";

/** Owns the one HTTP binding. Grant editors never mutate it through copy actions. */
export function useMcpHttpController(grants: readonly McpGrant[]) {
  const [savedConfig, setSavedConfig] = useState<McpHttpConfig | null>(null);
  const [settings, setSettings] = useState(defaultMcpHttpSettings);
  const [originsText, setOriginsText] = useState(() => formatMcpHttpOrigins(defaultMcpHttpSettings().allowedOrigins));
  const [dirty, setDirty] = useState(false);
  const dirtyRef = useRef(false);
  const [token, setToken] = useState("");
  const [runtime, setRuntime] = useState<McpHttpRuntimeStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [now, setNow] = useState(Date.now);
  // An explicit override is independent of the HTTP authorization identity.
  // Derive the automatic name during render so refresh/binding changes cannot
  // briefly expose a JSON entry named for the previous client.
  const [serverIdOverride, setServerId] = useState<string | null>(null);
  const serverId = serverIdOverride ?? ccSwitchServerIdForGrant(settings.clientId);
  const [toolTimeout, setToolTimeout] = useState(CC_SWITCH_DEFAULT_TOOL_TIMEOUT_SECONDS);
  const [copiedValue, setCopiedValue] = useState("");
  const [commandCopied, setCommandCopied] = useState(false);
  const [copying, setCopying] = useState(false);
  const gate = useRef(new KeyedRequestGate<"access" | "status" | "mutation" | "copy">()).current;
  const available = isBackendAvailable();
  const activeGrants = grants.filter(grant => mcpGrantIsActive(grant, now));
  const selectedGrant = activeGrants.find(grant => grant.clientId === settings.clientId) ?? null;
  const savedGrant = activeGrants.find(grant => grant.clientId === savedConfig?.clientId) ?? null;
  const running = runtime?.phase === "running" || runtime?.phase === "starting";
  const locked = busy || loading || copying || !available || runtime === null || running;
  const remote = isNonLoopbackMcpHost(settings.listenHost);
  const settingsValid = Boolean(selectedGrant && settings.listenHost.trim()
    && mcpHttpClientEndpoint(settings)) && (!remote || settings.allowRemote);
  const identityChanged = Boolean(savedConfig && settings.clientId !== savedConfig.clientId);
  // Unsaved identity/address edits must never display a usable-looking old token.
  const json = savedConfig && savedGrant && !dirty && !busy && !loading
    ? formatCcSwitchMcpJson(savedConfig, { serverId, token, toolTimeoutSeconds: toolTimeout }) : "";
  const copied = Boolean(json && copiedValue === json);

  function applyAccess(access: McpHttpAccessResponse, preserveDraft = false) {
    setSavedConfig(access.config);
    setToken(access.token ?? "");
    setCopiedValue("");
    setCommandCopied(false);
    if (!preserveDraft || !dirtyRef.current) {
      const next = mcpHttpSettingsFromConfig(access.config);
      setSettings(next);
      setOriginsText(formatMcpHttpOrigins(next.allowedOrigins));
      dirtyRef.current = false;
      setDirty(false);
    }
  }

  async function refreshAccess() {
    const request = gate.replace("access");
    setLoading(true);
    try {
      const access = await invokeBackend<McpHttpAccessResponse>("mcp_http_access_config", {});
      if (gate.isCurrent("access", request)) applyAccess(access, true);
    } catch (cause) {
      if (gate.isCurrent("access", request)) {
        setToken("");
        setError(formatError(cause));
      }
    } finally {
      if (gate.finish("access", request)) setLoading(false);
    }
  }

  async function refreshRuntime() {
    if (gate.isActive("mutation")) return;
    const request = gate.begin("status");
    if (request === null) return;
    try {
      const next = await invokeBackend<McpHttpRuntimeStatus>("mcp_http_runtime_status", {});
      if (gate.isCurrent("status", request)) setRuntime(next);
    } catch (cause) {
      if (gate.isCurrent("status", request)) {
        setRuntime(null);
        setError(formatError(cause));
      }
    } finally {
      gate.finish("status", request);
    }
  }

  useEffect(() => {
    if (available) {
      void refreshAccess();
      void refreshRuntime();
    } else setLoading(false);
    const timer = window.setInterval(() => {
      setNow(Date.now());
      if (available && !document.hidden) void refreshRuntime();
    }, 2_000);
    return () => { window.clearInterval(timer); gate.invalidateAll(); };
  }, [available, gate]);

  function update(patch: Partial<McpHttpConfigRequest>) {
    if (locked || gate.isActive("mutation")) return;
    setSettings(current => ({ ...current, ...patch }));
    dirtyRef.current = true;
    setDirty(true);
    setNotice("");
    setError("");
  }

  async function mutate(action: "save" | "start" | "stop" | "rotate") {
    if (!available || loading || runtime === null || gate.isActive("copy")) return;
    if (action !== "stop" && (locked || !settingsValid)) return;
    if (action === "rotate" && (dirty || !savedGrant)) return;
    if (action === "rotate" && token && !window.confirm(t("rotating-the-token-invalidates-the-old-http-access-configuration"))) return;
    const request = gate.begin("mutation");
    if (request === null) return;
    const current = () => gate.isCurrent("mutation", request);
    gate.invalidate("status");
    gate.invalidate("access");
    setBusy(true);
    setError("");
    setNotice("");
    try {
      if (action === "stop") {
        const next = await invokeBackend<McpHttpRuntimeStatus>("stop_mcp_http", {});
        if (current()) setRuntime(next);
      } else if (action === "rotate") {
        const access = await invokeBackend<McpHttpTokenResponse>("rotate_mcp_http_token", { expectedSettings: mcpHttpSettingsFromConfig(savedConfig!) });
        if (current()) { applyAccess(access); setNotice(t("new-token-generated-copy-the-client-access-configuration-again")); }
      } else {
        const access = await prepareMcpHttpAccess(invokeBackend,
          { ...settings, allowedOrigins: parseMcpHttpOrigins(originsText) }, dirty,
          action === "start", current, savedConfig ? mcpHttpSettingsFromConfig(savedConfig) : undefined);
        if (!access || !current()) return;
        applyAccess(access);
        if (action === "start") {
          const next = await invokeBackend<McpHttpRuntimeStatus>("start_mcp_http", { expectedSettings: mcpHttpSettingsFromConfig(access.config) });
          if (current()) { setRuntime(next); setNotice(t("service-started-access-json-is-ready-to-copy")); }
        } else setNotice(identityChanged ? t("binding-updated-the-old-token-is-invalid-a-new") : t("settings-saved-2"));
      }
    } catch (cause) {
      if (current()) {
        setError(formatError(cause));
        // Save may have invalidated the token before failing. Do not advertise
        // cached credentials; read back canonical state while keeping the draft.
        setToken("");
        await refreshAccess();
      }
    } finally {
      if (gate.finish("mutation", request)) {
        setBusy(false);
        void refreshRuntime();
      }
    }
  }

  async function copy(kind: "json" | "command") {
    if (busy || loading || dirty || !savedGrant || !savedConfig || gate.isActive("mutation")) return;
    const request = gate.begin("copy");
    if (request === null) return;
    const current = () => gate.isCurrent("copy", request);
    setCopying(true);
    try {
      if (!navigator.clipboard?.writeText) throw new Error(t("writing-to-the-system-clipboard-is-unavailable-in-this"));
      const access = await prepareMcpHttpAccess(invokeBackend, mcpHttpSettingsFromConfig(savedConfig), false, false, current);
      if (!access || !current()) return;
      if (kind === "json" && !access.token) throw new Error(t("the-http-grant-or-token-is-invalid-refresh-the"));
      const value = kind === "json" ? formatCcSwitchMcpJson(access.config, { serverId, token: access.token ?? undefined, toolTimeoutSeconds: toolTimeout }) : access.config.startCommand;
      if (!value) throw new Error(t("invalid-access-configuration-check-server-id-and-tool-timeout"));
      applyAccess(access, true);
      await navigator.clipboard.writeText(value);
      if (!current()) return;
      if (kind === "json") setCopiedValue(value); else setCommandCopied(true);
      setError("");
    } catch (cause) {
      if (current()) { setError(formatError(cause)); setToken(""); await refreshAccess(); }
    } finally { if (gate.finish("copy", request)) setCopying(false); }
  }

  async function refreshAfterGrantChange(invalidated: boolean) {
    if (invalidated) {
      setToken("");
      setNotice(t("the-http-grant-is-no-longer-valid-select-and"));
      gate.invalidate("status");
      setRuntime(null);
    }
    await refreshAccess();
    void refreshRuntime();
  }

  return {
    savedConfig, settings, originsText, dirty, token, runtime, busy, loading, copying, error, notice,
    serverId, toolTimeout, copied, commandCopied, activeGrants, selectedGrant, savedGrant,
    running, locked, remote, settingsValid, identityChanged, json, available,
    update, mutate, copy, refreshAfterGrantChange,
    refresh: () => { void refreshAccess(); void refreshRuntime(); },
    isMutating: () => gate.isActive("mutation") || gate.isActive("access") || gate.isActive("copy"),
    updateOrigins: (value: string) => { if (!locked) { setOriginsText(value); update({}); } },
    resetDraft: () => { if (!locked && savedConfig) { applyAccess({ config: savedConfig, token }); setNotice(""); setError(""); } },
    setServerId, setToolTimeout,
  };
}

export type McpHttpController = ReturnType<typeof useMcpHttpController>;
function formatError(error: unknown) { return error instanceof Error ? error.message : String(error); }
