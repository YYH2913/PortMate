import { useEffect, useRef, useState } from "react";
import { invokeBackend, isBackendAvailable } from "./api";
import { KeyedRequestGate } from "./keyed-request-gate";
import { mcpGrantIsActive } from "./mcp-grant-state";
import { CC_SWITCH_DEFAULT_SERVER_ID, CC_SWITCH_DEFAULT_TOOL_TIMEOUT_SECONDS, defaultMcpHttpSettings, formatCcSwitchMcpJson, formatMcpHttpOrigins, isNonLoopbackMcpHost, mcpHttpClientEndpoint, mcpHttpSettingsFromConfig, parseMcpHttpOrigins } from "./mcp-http-state";
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
  const [serverId, setServerId] = useState(CC_SWITCH_DEFAULT_SERVER_ID);
  const [toolTimeout, setToolTimeout] = useState(CC_SWITCH_DEFAULT_TOOL_TIMEOUT_SECONDS);
  const [copied, setCopied] = useState(false);
  const [commandCopied, setCommandCopied] = useState(false);
  const gate = useRef(new KeyedRequestGate<"access" | "status" | "mutation">()).current;
  const available = isBackendAvailable();
  const activeGrants = grants.filter(grant => mcpGrantIsActive(grant, now));
  const selectedGrant = activeGrants.find(grant => grant.clientId === settings.clientId) ?? null;
  const savedGrant = activeGrants.find(grant => grant.clientId === savedConfig?.clientId) ?? null;
  const running = runtime?.phase === "running" || runtime?.phase === "starting";
  const locked = busy || loading || !available || runtime === null || running;
  const remote = isNonLoopbackMcpHost(settings.listenHost);
  const settingsValid = Boolean(selectedGrant && settings.listenHost.trim()
    && mcpHttpClientEndpoint(settings)) && (!remote || settings.allowRemote);
  const identityChanged = Boolean(savedConfig && settings.clientId !== savedConfig.clientId);
  // Unsaved identity/address edits must never display a usable-looking old token.
  const json = savedConfig && savedGrant && !dirty && !busy && !loading
    ? formatCcSwitchMcpJson(savedConfig, { serverId, token, toolTimeoutSeconds: toolTimeout }) : "";

  function applyAccess(access: McpHttpAccessResponse, preserveDraft = false) {
    setSavedConfig(access.config);
    setToken(access.token ?? "");
    setCopied(false);
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
  useEffect(() => setCopied(false), [json]);

  function update(patch: Partial<McpHttpConfigRequest>) {
    if (locked || gate.isActive("mutation")) return;
    setSettings(current => ({ ...current, ...patch }));
    dirtyRef.current = true;
    setDirty(true);
    setNotice("");
    setError("");
  }

  async function mutate(action: "save" | "start" | "stop" | "rotate") {
    if (!available || loading || runtime === null) return;
    if (action !== "stop" && (locked || !settingsValid)) return;
    if (action === "rotate" && (dirty || !savedGrant)) return;
    if (action === "rotate" && token && !window.confirm("轮换 Token 后，旧的 HTTP 接入配置将失效。确认继续？")) return;
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
        const access = await invokeBackend<McpHttpTokenResponse>("rotate_mcp_http_token", {});
        if (current()) { applyAccess(access); setNotice("新 Token 已生成，请重新复制客户端接入配置。"); }
      } else {
        const access = await prepareMcpHttpAccess(invokeBackend,
          { ...settings, allowedOrigins: parseMcpHttpOrigins(originsText) }, dirty,
          action === "start", current);
        if (!access || !current()) return;
        applyAccess(access);
        if (action === "start") {
          const next = await invokeBackend<McpHttpRuntimeStatus>("start_mcp_http", {});
          if (current()) { setRuntime(next); setNotice("服务已启动，可以复制接入 JSON。"); }
        } else setNotice(identityChanged ? "绑定已更新，旧 Token 已失效。启动时将生成新 Token。" : "配置已保存。");
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
    if (busy || loading || dirty || !savedGrant) return;
    const value = kind === "json" ? json : savedConfig?.startCommand;
    if (!value) return;
    try {
      if (!navigator.clipboard?.writeText) throw new Error("当前环境不支持写入系统剪贴板。");
      await navigator.clipboard.writeText(value);
      if (kind === "json") setCopied(true); else setCommandCopied(true);
      setError("");
    } catch (cause) { setError(formatError(cause)); }
  }

  async function refreshAfterGrantChange(invalidated: boolean) {
    if (invalidated) {
      setToken("");
      setNotice("当前 HTTP 授权已失效。请选择新的授权并保存，旧身份不会自动替换。");
      gate.invalidate("status");
      setRuntime(null);
    }
    await refreshAccess();
    void refreshRuntime();
  }

  return {
    savedConfig, settings, originsText, dirty, token, runtime, busy, loading, error, notice,
    serverId, toolTimeout, copied, commandCopied, activeGrants, selectedGrant, savedGrant,
    running, locked, remote, settingsValid, identityChanged, json, available,
    update, mutate, copy, refreshAfterGrantChange,
    refresh: () => { void refreshAccess(); void refreshRuntime(); },
    isMutating: () => gate.isActive("mutation") || gate.isActive("access"),
    updateOrigins: (value: string) => { if (!locked) { setOriginsText(value); update({}); } },
    resetDraft: () => { if (!locked && savedConfig) { applyAccess({ config: savedConfig, token }); setNotice(""); setError(""); } },
    setServerId, setToolTimeout,
  };
}

export type McpHttpController = ReturnType<typeof useMcpHttpController>;
function formatError(error: unknown) { return error instanceof Error ? error.message : String(error); }
