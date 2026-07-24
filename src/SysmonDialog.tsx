import { useEffect, useRef, useState } from "react";
import { LoaderCircle, RefreshCw, X } from "lucide-react";
import { invokeBackend } from "./api";
import { formatBytes, formatEventClock } from "./display-formatters";
import { KeyedRequestGate } from "./keyed-request-gate";
import { mergeSysmonHistory, normalizeSysmonHistory, sysmonTrendMax, sysmonTrendValue } from "./sysmon-history";
import { formatSysmonNetworkAddresses, orderedSysmonNetworkAddresses } from "./sysmon-network-addresses";
import type { SysmonTrendMode } from "./sysmon-history";
import type { SessionSummary, SysmonSnapshot } from "./types";

export default function SysmonDialog({
  session,
  onClose,
}: {
  session: SessionSummary;
  onClose: () => void;
}) {
  const [snapshot, setSnapshot] = useState<SysmonSnapshot | null>(null);
  const [history, setHistory] = useState<SysmonSnapshot[]>([]);
  const [tab, setTab] = useState<"processes" | "disks" | "network" | "trends">("processes");
  const [trendMode, setTrendMode] = useState<SysmonTrendMode>("usage");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [historyError, setHistoryError] = useState("");
  const requestGate = useRef(new KeyedRequestGate<"history" | "snapshot">());

  useEffect(() => {
    requestGate.current.invalidateAll();
    setSnapshot(null);
    setHistory([]);
    setError("");
    setHistoryError("");
    void loadSysmonHistory();
    void refreshSysmon();
    return () => requestGate.current.invalidateAll();
  }, [session.profile.id]);

  async function loadSysmonHistory() {
    const gate = requestGate.current;
    const token = gate.begin("history");
    if (token === null) return;
    try {
      const loaded = await invokeBackend<SysmonSnapshot[]>("list_sysmon_history", {
        sessionId: session.profile.id,
        limit: 120,
      });
      if (!gate.isCurrent("history", token)) return;
      setHistory((current) => normalizeSysmonHistory([...current, ...loaded], session.profile.id, 120));
      setHistoryError("");
    } catch (error) {
      if (gate.isCurrent("history", token)) setHistoryError(formatSysmonError(error));
    } finally {
      gate.finish("history", token);
    }
  }

  async function refreshSysmon() {
    const gate = requestGate.current;
    const token = gate.begin("snapshot");
    if (token === null) return;
    setBusy(true);
    setError("");
    try {
      const next = await invokeBackend<SysmonSnapshot>("refresh_sysmon", { sessionId: session.profile.id });
      if (!gate.isCurrent("snapshot", token)) return;
      setSnapshot(next);
      setHistory((current) => mergeSysmonHistory(current, next, 120));
    } catch (error) {
      if (gate.isCurrent("snapshot", token)) setError(formatSysmonError(error));
    } finally {
      const current = gate.isCurrent("snapshot", token);
      gate.finish("snapshot", token);
      if (current) setBusy(false);
    }
  }

  const processes = snapshot?.processes ?? [];
  const disks = snapshot?.disks ?? [];
  const interfaces = snapshot?.networkInterfaces ?? [];
  const loadAverage = snapshot?.loadAverage ?? [0, 0, 0];
  const memoryUsed = snapshot ? Math.max(0, snapshot.memoryTotalBytes - snapshot.memoryAvailableBytes) : 0;
  const scope = isSshLikeSession(session) ? "远端主机" : "本机";

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section className="wind-dialog sysmon-dialog">
        <header className="dialog-title">
          <span className="app-icon" />
          <div className="sysmon-title">
            <strong>Sysmon</strong>
            <small>{session.profile.name} · {scope}</small>
          </div>
          <button title="关闭 Sysmon" aria-label="关闭 Sysmon" onClick={onClose}><X size={20} /></button>
        </header>
        <div className="sysmon-content">
          <dl className="sysmon-summary">
            <div><dt>CPU</dt><dd>{snapshot ? `${snapshot.cpuPercent.toFixed(1)}%` : "-"}</dd></div>
            <div>
              <dt>内存</dt>
              <dd>{snapshot ? `${snapshot.memoryPercent.toFixed(1)}%` : "-"}</dd>
              <small>{snapshot?.memoryTotalBytes ? `${formatBytes(memoryUsed)} / ${formatBytes(snapshot.memoryTotalBytes)}` : "-"}</small>
            </div>
            <div><dt>负载</dt><dd>{snapshot ? loadAverage.map((value) => value.toFixed(2)).join(" · ") : "-"}</dd></div>
            <div><dt>接收</dt><dd>{snapshot ? `${snapshot.rxKbps.toFixed(1)} KiB/s` : "-"}</dd></div>
            <div><dt>发送</dt><dd>{snapshot ? `${snapshot.txKbps.toFixed(1)} KiB/s` : "-"}</dd></div>
            <div><dt>运行时间</dt><dd>{snapshot ? formatSysmonUptime(snapshot.uptimeSeconds) : "-"}</dd></div>
          </dl>

          <nav className="sysmon-tabs" aria-label="Sysmon 详情">
            <button className={tab === "processes" ? "active" : ""} onClick={() => setTab("processes")}>进程 <span>{processes.length}</span></button>
            <button className={tab === "disks" ? "active" : ""} onClick={() => setTab("disks")}>磁盘 <span>{disks.length}</span></button>
            <button className={tab === "network" ? "active" : ""} onClick={() => setTab("network")}>网络 <span>{interfaces.length}</span></button>
            <button className={tab === "trends" ? "active" : ""} onClick={() => setTab("trends")}>趋势 <span>{history.length}</span></button>
          </nav>

          <div className="sysmon-table-wrap">
            {tab === "trends" ? (
              <SysmonTrendView history={history} mode={trendMode} onModeChange={setTrendMode} error={historyError} />
            ) : null}
            {tab === "processes" ? (
              <table className="sysmon-table sysmon-process-table">
                <thead><tr><th>PID</th><th>进程</th><th>CPU</th><th>内存</th><th>RSS</th></tr></thead>
                <tbody>
                  {processes.map((process) => (
                    <tr key={process.pid}>
                      <td>{process.pid}</td><td title={process.name}>{process.name}</td><td>{process.cpuPercent.toFixed(1)}%</td><td>{process.memoryPercent.toFixed(1)}%</td><td>{formatBytes(process.rssBytes)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : null}
            {tab === "disks" ? (
              <table className="sysmon-table sysmon-disk-table">
                <thead><tr><th>挂载点</th><th>文件系统</th><th>使用率</th><th>可用</th><th>总计</th></tr></thead>
                <tbody>
                  {disks.map((disk) => (
                    <tr key={`${disk.filesystem}-${disk.mountPoint}`}>
                      <td title={disk.mountPoint}>{disk.mountPoint}</td>
                      <td title={disk.filesystem}>{disk.filesystem}</td>
                      <td><div className="sysmon-usage"><span style={{ width: `${Math.min(100, Math.max(0, disk.usedPercent))}%` }} /><b>{disk.usedPercent.toFixed(1)}%</b></div></td>
                      <td>{formatBytes(disk.availableBytes)}</td><td>{formatBytes(disk.totalBytes)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : null}
            {tab === "network" ? (
              <table className="sysmon-table sysmon-network-table">
                <thead><tr><th>接口</th><th>IP 地址</th><th>接收速率</th><th>发送速率</th><th>已接收</th><th>已发送</th></tr></thead>
                <tbody>
                  {interfaces.map((item) => (
                    <tr key={item.name}>
                      <td title={item.name}>{item.name}</td><td title={orderedSysmonNetworkAddresses(item.addresses).join(" / ")}>{formatSysmonNetworkAddresses(item.addresses)}</td><td>{item.rxKbps.toFixed(1)} KiB/s</td><td>{item.txKbps.toFixed(1)} KiB/s</td><td>{formatBytes(item.rxBytes)}</td><td>{formatBytes(item.txBytes)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : null}
            {snapshot && ((tab === "processes" && !processes.length) || (tab === "disks" && !disks.length) || (tab === "network" && !interfaces.length)) ? (
              <div className="sysmon-empty">当前采样没有可显示的{tab === "processes" ? "进程" : tab === "disks" ? "磁盘" : "网络接口"}明细</div>
            ) : null}
            {!snapshot && !error && tab !== "trends" ? <div className="sysmon-empty loading"><LoaderCircle size={18} />正在采样</div> : null}
          </div>
          {error ? <div className="utility-error">{error}</div> : null}
        </div>
        <footer className="sysmon-actions">
          <span>{snapshot ? `采样时间 ${formatDateTime(snapshot.ts)}` : scope}</span>
          <button type="button" onClick={() => void refreshSysmon()} disabled={busy}>
            <RefreshCw size={14} className={busy ? "sysmon-refresh-icon loading" : "sysmon-refresh-icon"} />刷新
          </button>
          <button type="button" onClick={onClose}>关闭</button>
        </footer>
      </section>
    </div>
  );
}

function SysmonTrendView({
  history,
  mode,
  onModeChange,
  error,
}: {
  history: SysmonSnapshot[];
  mode: SysmonTrendMode;
  onModeChange: (mode: SysmonTrendMode) => void;
  error: string;
}) {
  const latest = history[history.length - 1];
  const first = history[0];
  const usageMode = mode === "usage";

  return (
    <section className="sysmon-trend-view">
      <header className="sysmon-trend-toolbar">
        <div className="sysmon-trend-modes" role="group" aria-label="趋势指标">
          <button type="button" className={usageMode ? "active" : ""} onClick={() => onModeChange("usage")}>利用率</button>
          <button type="button" className={!usageMode ? "active" : ""} onClick={() => onModeChange("network")}>网络</button>
        </div>
        <div className="sysmon-trend-legend">
          <span className={usageMode ? "cpu" : "rx"}>{usageMode ? "CPU" : "RX"} <b>{latest ? formatSysmonTrendValue(latest, mode, 0) : "-"}</b></span>
          <span className={usageMode ? "memory" : "tx"}>{usageMode ? "内存" : "TX"} <b>{latest ? formatSysmonTrendValue(latest, mode, 1) : "-"}</b></span>
        </div>
      </header>
      <div className="sysmon-trend-stage">
        <SysmonTrendCanvas history={history} mode={mode} />
        {!history.length ? <div className="sysmon-trend-empty">暂无历史样本</div> : null}
      </div>
      <footer className="sysmon-trend-range">
        <span>{first ? formatEventClock(first.ts) : "--:--:--"}</span>
        <b>{history.length} 个样本</b>
        <span>{latest ? formatEventClock(latest.ts) : "--:--:--"}</span>
      </footer>
      {error ? <div className="utility-error">{error}</div> : null}
    </section>
  );
}

function SysmonTrendCanvas({ history, mode }: { history: SysmonSnapshot[]; mode: SysmonTrendMode }) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const draw = () => {
      const width = Math.max(1, Math.floor(canvas.clientWidth));
      const height = Math.max(1, Math.floor(canvas.clientHeight));
      const scale = Math.min(2, window.devicePixelRatio || 1);
      canvas.width = Math.floor(width * scale);
      canvas.height = Math.floor(height * scale);
      const context = canvas.getContext("2d");
      if (!context) return;
      context.setTransform(scale, 0, 0, scale, 0, 0);
      context.clearRect(0, 0, width, height);

      const plot = { left: 44, right: 12, top: 14, bottom: 22 };
      const plotWidth = Math.max(1, width - plot.left - plot.right);
      const plotHeight = Math.max(1, height - plot.top - plot.bottom);
      const maximum = sysmonTrendMax(history, mode);
      context.font = '10px "JetBrains Mono", monospace';
      context.lineWidth = 1;
      context.textAlign = "right";
      context.textBaseline = "middle";

      for (let index = 0; index <= 4; index += 1) {
        const ratio = index / 4;
        const y = plot.top + plotHeight * ratio;
        context.strokeStyle = index === 4 ? "#344252" : "#26323f";
        context.beginPath();
        context.moveTo(plot.left, y + 0.5);
        context.lineTo(width - plot.right, y + 0.5);
        context.stroke();
        context.fillStyle = "#8da0b3";
        context.fillText(formatSysmonTrendAxis(maximum * (1 - ratio), mode), plot.left - 7, y);
      }

      if (!history.length) return;
      const timestamps = history.map((snapshot) => Date.parse(snapshot.ts));
      const start = timestamps[0];
      const span = Math.max(1, timestamps[timestamps.length - 1] - start);
      const xAt = (index: number) => history.length === 1
        ? plot.left + plotWidth / 2
        : plot.left + ((timestamps[index] - start) / span) * plotWidth;
      const colors = mode === "usage" ? ["#5eead4", "#f4b860"] : ["#68a7ff", "#e879f9"];

      for (const series of [0, 1] as const) {
        context.strokeStyle = colors[series];
        context.lineWidth = 1.6;
        context.lineJoin = "round";
        context.lineCap = "round";
        context.beginPath();
        history.forEach((snapshot, index) => {
          const value = Math.min(maximum, sysmonTrendValue(snapshot, mode, series));
          const x = xAt(index);
          const y = plot.top + plotHeight * (1 - value / maximum);
          if (index === 0) context.moveTo(x, y);
          else context.lineTo(x, y);
        });
        context.stroke();

        const lastIndex = history.length - 1;
        const lastValue = Math.min(maximum, sysmonTrendValue(history[lastIndex], mode, series));
        context.fillStyle = colors[series];
        context.beginPath();
        context.arc(xAt(lastIndex), plot.top + plotHeight * (1 - lastValue / maximum), 2.5, 0, Math.PI * 2);
        context.fill();
      }
    };

    draw();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(draw);
    observer?.observe(canvas);
    window.addEventListener("resize", draw);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", draw);
    };
  }, [history, mode]);

  return (
    <canvas
      ref={canvasRef}
      className="sysmon-trend-canvas"
      role="img"
      aria-label={`${mode === "usage" ? "CPU 和内存利用率" : "网络接收和发送速率"}趋势，${history.length} 个样本`}
    />
  );
}

function isSshLikeSession(session: SessionSummary) {
  return session.profile.connection.kind === "ssh" || session.profile.connection.kind === "tmux";
}

function formatSysmonUptime(seconds: number) {
  const wholeSeconds = Math.max(0, Math.trunc(seconds));
  const days = Math.floor(wholeSeconds / 86_400);
  const hours = Math.floor((wholeSeconds % 86_400) / 3_600);
  const minutes = Math.floor((wholeSeconds % 3_600) / 60);
  if (days) return `${days}d ${hours}h`;
  if (hours) return `${hours}h ${minutes}m`;
  if (minutes) return `${minutes}m ${wholeSeconds % 60}s`;
  return `${wholeSeconds}s`;
}

function formatSysmonTrendValue(snapshot: SysmonSnapshot, mode: SysmonTrendMode, series: 0 | 1) {
  const value = sysmonTrendValue(snapshot, mode, series);
  return mode === "usage" ? `${value.toFixed(1)}%` : formatSysmonTrendRate(value);
}

function formatSysmonTrendAxis(value: number, mode: SysmonTrendMode) {
  if (mode === "usage") return `${Math.round(value)}%`;
  if (value >= 1024) return `${(value / 1024).toFixed(value >= 10_240 ? 0 : 1)}M`;
  return `${Math.round(value)}K`;
}

function formatSysmonTrendRate(kibibytesPerSecond: number) {
  if (kibibytesPerSecond >= 1024) {
    return `${(kibibytesPerSecond / 1024).toFixed(1)} MiB/s`;
  }
  return `${kibibytesPerSecond.toFixed(1)} KiB/s`;
}

function formatDateTime(value?: string | null) {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

function formatSysmonError(error: unknown) {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}
