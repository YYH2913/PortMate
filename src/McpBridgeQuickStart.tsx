import { t, useLocale } from "./i18n";
import { useEffect, useRef, useState } from "react";
import { LoaderCircle, Play, Radio } from "lucide-react";
import { invokeBackend, isBackendAvailable } from "./api";
import { KeyedRequestGate } from "./keyed-request-gate";
import type { McpHttpRuntimeStatus } from "./types";

/** The shortcut starts the saved HTTP service; it never creates tokens or grants. */
export default function McpBridgeQuickStart({ paused, onOpen, onError }: {
  paused: boolean;
  onOpen: () => void;
  onError: (message: string) => void;
}) {
  useLocale();
  const [status, setStatus] = useState<McpHttpRuntimeStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const pausedRef = useRef(paused);
  pausedRef.current = paused;
  const gate = useRef(new KeyedRequestGate<"status" | "start">()).current;
  const available = isBackendAvailable();

  useEffect(() => () => gate.invalidateAll(), [gate]);
  useEffect(() => {
    if (paused || !available) return;
    const refresh = async () => {
      if (gate.isActive("start") || document.hidden) return;
      const token = gate.begin("status");
      if (token === null) return;
      try {
        const next = await invokeBackend<McpHttpRuntimeStatus>("mcp_http_runtime_status", {});
        if (gate.isCurrent("status", token)) setStatus(next);
      } catch {
        if (gate.isCurrent("status", token)) setStatus(null);
      } finally {
        gate.finish("status", token);
      }
    };
    void refresh();
    const timer = window.setInterval(() => void refresh(), 5_000);
    return () => {
      window.clearInterval(timer);
      gate.invalidate("status");
    };
  }, [available, gate, paused]);

  async function start() {
    if (paused || !available) return;
    const token = gate.begin("start");
    if (token === null) return;
    gate.invalidate("status");
    setBusy(true);
    try {
      // Recheck before starting: the management dialog or another window may
      // already have started the service since the last status poll.
      let next = await invokeBackend<McpHttpRuntimeStatus>("mcp_http_runtime_status", {});
      if (!gate.isCurrent("start", token)) return;
      if (next.phase === "running" || next.phase === "starting") {
        setStatus(next);
        if (!pausedRef.current) onOpen();
        return;
      }
      next = await invokeBackend<McpHttpRuntimeStatus>("start_mcp_http", {});
      if (!gate.isCurrent("start", token)) return;
      setStatus(next);
      if (next.phase !== "running") throw new Error(next.message || t("mcp-bridge-has-not-started-check-the-service-settings"));
    } catch (error) {
      if (!gate.isCurrent("start", token)) return;
      setStatus({ phase: "failed" });
      // Do not replace a dialog opened during startup or surface a notice
      // over a screen that the user has since locked.
      if (!pausedRef.current) {
        onOpen();
        onError(t("check-the-saved-configuration-and-token-on-the-mcp", [error instanceof Error ? error.message : String(error)]));
      }
    } finally {
      if (gate.finish("start", token)) setBusy(false);
    }
  }

  const running = status?.phase === "running";
  const label = busy ? t("starting") : running ? t("running") : status?.phase === "starting" ? t("starting") : status?.phase === "failed" ? t("retry") : t("start");
  return (
    <button
      type="button"
      className={`menu-mcp-start${running ? " active" : ""}`}
      disabled={!available || busy || paused}
      aria-label={busy ? t("starting-mcp-bridge") : running ? t("manage-mcp-bridge") : t("quick-start-mcp-bridge")}
      aria-busy={busy}
      title={!available ? t("mcp-bridge-requires-the-desktop-backend") : running ? t("mcp-bridge-running-click-to-manage", [status.endpoint ?? ""]) : t("start-mcp-bridge-using-the-saved-http-settings")}
      onClick={() => void start()}
    >
      {busy ? <LoaderCircle size={13} /> : running ? <Radio size={13} /> : <Play size={13} />}
      <span>MCP Bridge</span><small>{label}</small>
    </button>
  );
}
