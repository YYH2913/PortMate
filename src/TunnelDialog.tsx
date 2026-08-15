import { useEffect, useRef, useState } from "react";
import type { FormEvent, ReactNode } from "react";
import { Plus, RefreshCw, Square, Trash2, X } from "lucide-react";
import { invokeBackend } from "./api";
import { formatBytes } from "./display-formatters";
import { KeyedRequestGate } from "./keyed-request-gate";
import {
  canAddTunnel,
  isValidTunnelHostInput,
  isValidTunnelRouteRules,
  MAX_TUNNEL_HOST_CHARACTERS,
  MAX_TUNNEL_ROUTE_RULES,
  normalizeTunnelRouteHost,
  parseTunnelPort,
} from "./tunnel-state";
import type { SessionSummary, TunnelRouteRule, TunnelSpec, TunnelStatus } from "./types";

export default function TunnelDialog({
  session,
  onClose,
  onDone,
}: {
  session: SessionSummary;
  onClose: () => void;
  onDone: (message: string) => void;
}) {
  const [mode, setMode] = useState<TunnelSpec["mode"]>("local");
  const [bindHost, setBindHost] = useState("127.0.0.1");
  const [bindPort, setBindPort] = useState("10022");
  const [targetHost, setTargetHost] = useState("127.0.0.1");
  const [targetPort, setTargetPort] = useState("22");
  const [routeRules, setRouteRules] = useState<TunnelRouteRule[]>([]);
  const [tunnels, setTunnels] = useState<TunnelStatus[]>([]);
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(false);
  const [stoppingIds, setStoppingIds] = useState<Set<string>>(() => new Set());
  const [error, setError] = useState("");
  const refreshGate = useRef(new KeyedRequestGate<"tunnels">());
  const createGate = useRef(new KeyedRequestGate<"create">());
  const stopGate = useRef(new KeyedRequestGate<string>());

  useEffect(() => {
    refreshGate.current.invalidate("tunnels");
    setTunnels([]);
    stopGate.current.invalidateAll();
    setStoppingIds(new Set());
    void refreshTunnels();
    const timer = window.setInterval(() => void refreshTunnels(true), 2_000);
    return () => {
      refreshGate.current.invalidate("tunnels");
      createGate.current.invalidateAll();
      stopGate.current.invalidateAll();
      window.clearInterval(timer);
    };
  }, [session.profile.id]);

  async function refreshTunnels(quiet = false) {
    const gate = refreshGate.current;
    const token = gate.begin("tunnels");
    if (token === null) return;
    if (!quiet) {
      setLoading(true);
      setError("");
    }
    try {
      const next = await invokeBackend<TunnelStatus[]>("list_tunnels", { sessionId: session.profile.id });
      if (!gate.isCurrent("tunnels", token)) return;
      setTunnels(next);
    } catch (error) {
      if (gate.isCurrent("tunnels", token) && !quiet) setError(formatTunnelError(error));
    } finally {
      const current = gate.isCurrent("tunnels", token);
      gate.finish("tunnels", token);
      if (current && !quiet) setLoading(false);
    }
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    setError("");
    const normalizedBindPort = parseTunnelPort(bindPort, true);
    const normalizedTargetPort = mode === "dynamic" ? 0 : parseTunnelPort(targetPort, false);
    if (normalizedBindPort === null || normalizedTargetPort === null) {
      setError("端口必须是 0 到 65535 之间的整数，目标端口不能为 0。");
      return;
    }
    const gate = createGate.current;
    const token = gate.begin("create");
    if (token === null) return;
    setBusy(true);
    try {
      const tunnel = await invokeBackend<TunnelSpec>("create_tunnel", {
        request: {
          sessionId: session.profile.id,
          mode,
          bindHost,
          bindPort: normalizedBindPort,
          targetHost: mode === "dynamic" ? "" : targetHost,
          targetPort: normalizedTargetPort,
          routeRules: mode === "dynamic" ? normalizedRouteRules(routeRules) : [],
        },
      });
      if (!gate.isCurrent("create", token)) return;
      refreshGate.current.invalidate("tunnels");
      setTunnels((current) => mergeTunnels(current, emptyTunnelStatus(tunnel)));
      onDone(`已创建 ${mode} tunnel：${tunnel.label}`);
    } catch (error) {
      if (gate.isCurrent("create", token)) setError(formatTunnelError(error));
    } finally {
      if (gate.finish("create", token)) setBusy(false);
    }
  }

  async function stopTunnel(tunnel: TunnelStatus) {
    const tunnelId = tunnel.spec.id;
    const token = stopGate.current.begin(tunnelId);
    if (token === null) return;
    setStoppingIds((current) => new Set(current).add(tunnelId));
    setError("");
    try {
      await invokeBackend<TunnelStatus>("stop_tunnel", { tunnelId });
      if (!stopGate.current.isCurrent(tunnelId, token)) return;
      refreshGate.current.invalidate("tunnels");
      setTunnels((current) => current.filter((item) => item.spec.id !== tunnelId));
      onDone(`已停止 tunnel：${tunnel.spec.label}`);
    } catch (error) {
      if (stopGate.current.isCurrent(tunnelId, token)) setError(formatTunnelError(error));
    } finally {
      if (stopGate.current.finish(tunnelId, token)) {
        setStoppingIds((current) => {
          const next = new Set(current);
          next.delete(tunnelId);
          return next;
        });
      }
    }
  }

  const sessionTunnels = tunnels.filter((tunnel) => tunnel.spec.enabled);
  const savedEnabledTunnelCount = session.profile.connection.kind === "ssh"
    || session.profile.connection.kind === "tmux"
    ? session.profile.connection.tunnels.filter((tunnel) => tunnel.enabled).length
    : 0;
  const activeTunnelCount = Math.max(sessionTunnels.length, savedEnabledTunnelCount);
  const tunnelLimitReached = !canAddTunnel(activeTunnelCount);
  const formValid = isValidTunnelHostInput(bindHost)
    && parseTunnelPort(bindPort, true) !== null
    && (mode === "dynamic"
      ? isValidTunnelRouteRules(normalizedRouteRules(routeRules))
      : isValidTunnelHostInput(targetHost) && parseTunnelPort(targetPort, false) !== null);

  function addRouteRule() {
    if (routeRules.length >= MAX_TUNNEL_ROUTE_RULES) return;
    setRouteRules((current) => [...current, { host: "", port: null }]);
  }

  function updateRouteRule(index: number, patch: Partial<TunnelRouteRule>) {
    setRouteRules((current) => current.map((rule, ruleIndex) => ruleIndex === index
      ? { ...rule, ...patch }
      : rule));
  }

  function removeRouteRule(index: number) {
    setRouteRules((current) => current.filter((_, ruleIndex) => ruleIndex !== index));
  }

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <form className="wind-dialog utility-dialog" onSubmit={submit}>
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>端口转发</strong>
          <button type="button" onClick={onClose}><X size={20} /></button>
        </header>
        <section className="utility-content">
          <DialogField label="会话:">
            <input value={session.profile.name} readOnly />
          </DialogField>
          <DialogField label="模式:">
            <select value={mode} onChange={(event) => setMode(event.target.value as TunnelSpec["mode"])}>
              <option value="local">local</option>
              <option value="dynamic">dynamic SOCKS5</option>
              <option value="remote">remote</option>
            </select>
          </DialogField>
          <DialogField label="监听:">
            <input maxLength={MAX_TUNNEL_HOST_CHARACTERS} value={bindHost} onChange={(event) => setBindHost(event.target.value)} />
          </DialogField>
          <DialogField label="端口:">
            <input type="number" min={0} max={65_535} step={1} inputMode="numeric" value={bindPort} onChange={(event) => setBindPort(event.target.value)} />
          </DialogField>
          {mode !== "dynamic" ? (
            <>
              <DialogField label="目标:">
                <input maxLength={MAX_TUNNEL_HOST_CHARACTERS} value={targetHost} onChange={(event) => setTargetHost(event.target.value)} />
              </DialogField>
              <DialogField label="目标端口:">
                <input type="number" min={1} max={65_535} step={1} inputMode="numeric" value={targetPort} onChange={(event) => setTargetPort(event.target.value)} />
              </DialogField>
            </>
          ) : (
            <div className="tunnel-routes" aria-label="指定目标路由">
              <header>
                <div>
                  <strong>目标路由</strong>
                  <small>{routeRules.length ? `仅允许 ${routeRules.length} 条规则` : "允许全部 SOCKS5 目标"}</small>
                </div>
                <button
                  type="button"
                  onClick={addRouteRule}
                  disabled={routeRules.length >= MAX_TUNNEL_ROUTE_RULES}
                  title="添加目标路由"
                  aria-label="添加目标路由"
                >
                  <Plus size={14} />
                </button>
              </header>
              {routeRules.length ? (
                <div className="tunnel-route-list">
                  {routeRules.map((rule, index) => (
                    <div className="tunnel-route-row" key={index}>
                      <input
                        aria-label={`路由目标 ${index + 1}`}
                        maxLength={MAX_TUNNEL_HOST_CHARACTERS}
                        placeholder="host / *.domain / CIDR"
                        value={rule.host}
                        onChange={(event) => updateRouteRule(index, { host: event.target.value })}
                        onBlur={() => updateRouteRule(index, { host: normalizeTunnelRouteHost(rule.host) })}
                      />
                      <input
                        aria-label={`路由端口 ${index + 1}`}
                        type="number"
                        min={1}
                        max={65_535}
                        step={1}
                        inputMode="numeric"
                        placeholder="全部"
                        value={rule.port ?? ""}
                        onChange={(event) => updateRouteRule(index, {
                          port: event.target.value === "" ? null : Number(event.target.value),
                        })}
                      />
                      <button
                        type="button"
                        onClick={() => removeRouteRule(index)}
                        title="删除目标路由"
                        aria-label={`删除目标路由 ${index + 1}`}
                      >
                        <Trash2 size={14} />
                      </button>
                    </div>
                  ))}
                </div>
              ) : null}
            </div>
          )}
          <div className="tunnel-panel">
            <header>
              <strong>运行中</strong>
              <button type="button" onClick={() => void refreshTunnels()} disabled={loading} title="刷新 tunnel 列表">
                <RefreshCw size={14} />
              </button>
            </header>
            {sessionTunnels.length ? (
              <div className="tunnel-list">
                {sessionTunnels.map((tunnel) => (
                  <div key={tunnel.spec.id} className={`tunnel-row ${tunnel.lastError ? "degraded" : ""}`}>
                    <div>
                      <strong>{tunnel.spec.label}</strong>
                      <small>{tunnel.spec.mode} · {tunnel.spec.bindHost}:{tunnel.spec.bindPort}{tunnel.spec.mode === "dynamic" ? dynamicRouteSummary(tunnel.spec.routeRules) : ` -> ${tunnel.spec.targetHost}:${tunnel.spec.targetPort}`}</small>
                      <small>
                        active {tunnel.activeConnections} · total {tunnel.totalConnections} · TCP→SSH {formatBytes(tunnel.tcpToSshBytes)} · SSH→TCP {formatBytes(tunnel.sshToTcpBytes)}
                      </small>
                      {tunnel.lastError ? <small className="tunnel-error">{tunnel.lastError}</small> : null}
                    </div>
                    <button type="button" onClick={() => void stopTunnel(tunnel)} disabled={stoppingIds.has(tunnel.spec.id)} title="停止 tunnel">
                      <Square size={13} />
                    </button>
                  </div>
                ))}
              </div>
            ) : (
              <div className="empty-pane top">{loading ? "正在读取 tunnel" : "没有运行中的 tunnel"}</div>
            )}
          </div>
          {error ? <div className="utility-error">{error}</div> : null}
        </section>
        <footer className="utility-actions">
          <button type="button" onClick={onClose}>取消</button>
          <button type="submit" disabled={busy || tunnelLimitReached || !formValid} title={tunnelLimitReached ? "已达到 64 条 tunnel 上限" : "创建 tunnel"}>{busy ? "创建中" : "创建"}</button>
        </footer>
      </form>
    </div>
  );
}

function normalizedRouteRules(rules: TunnelRouteRule[]): TunnelRouteRule[] {
  return rules.map((rule) => ({ ...rule, host: normalizeTunnelRouteHost(rule.host) }));
}

function dynamicRouteSummary(rules: TunnelRouteRule[]): string {
  if (!rules.length) return " -> all targets";
  return ` -> ${rules.length} target rule${rules.length === 1 ? "" : "s"}`;
}

function DialogField({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="dialog-field">
      <span>{label}</span>
      {children}
    </label>
  );
}

function mergeTunnels(current: TunnelStatus[], saved: TunnelStatus): TunnelStatus[] {
  const index = current.findIndex((tunnel) => tunnel.spec.id === saved.spec.id);
  if (index < 0) return [...current, saved];
  return current.map((tunnel, itemIndex) => itemIndex === index ? saved : tunnel);
}

function emptyTunnelStatus(spec: TunnelSpec): TunnelStatus {
  return {
    spec,
    activeConnections: 0,
    totalConnections: 0,
    tcpToSshBytes: 0,
    sshToTcpBytes: 0,
    lastActivity: null,
    lastError: null,
  };
}

function formatTunnelError(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}
