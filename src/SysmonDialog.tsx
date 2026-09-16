import { t, useLocale, localizeDiagnostic } from "./i18n";
import { useEffect, useRef, useState } from "react";
import { LoaderCircle, RefreshCw, X } from "lucide-react";
import { formatBytes, formatEventClock } from "./display-formatters";
import { sysmonTrendMax, sysmonTrendValue } from "./sysmon-history";
import { loadSysmonLiveHistory, refreshSysmonLive, useSysmonLivePolling, useSysmonLiveState } from "./sysmon-live-state";
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
  useLocale();
  const [tab, setTab] = useState<"processes" | "disks" | "network" | "trends">("processes");
  const [trendMode, setTrendMode] = useState<SysmonTrendMode>("usage");
  const remote = isSshLikeSession(session);
  const canSample = !remote || session.runtime.status === "connected";
  const { snapshot, history, busy, historyBusy, error, historyError } = useSysmonLiveState(session.profile.id);
  useSysmonLivePolling(session.profile.id, canSample);

  useEffect(() => {
    void loadSysmonLiveHistory(session.profile.id, 120);
  }, [session.profile.id]);

  const processes = snapshot?.processes ?? [];
  const disks = snapshot?.disks ?? [];
  const interfaces = snapshot?.networkInterfaces ?? [];
  const loadAverage = snapshot?.loadAverage ?? [0, 0, 0];
  const memoryUsed = snapshot ? Math.max(0, snapshot.memoryTotalBytes - snapshot.memoryAvailableBytes) : 0;
  const scope = remote ? t("remote-host") : t("local-host");

  return (
    <div className="dialog-backdrop utility-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section className="wind-dialog sysmon-dialog">
        <header className="dialog-title">
          <span className="app-icon" />
          <div className="sysmon-title">
            <strong>Sysmon</strong>
            <small>{session.profile.name} · {scope}</small>
          </div>
          <button title={t("close-sysmon")} aria-label={t("close-sysmon")} onClick={onClose}><X size={20} /></button>
        </header>
        <div className="sysmon-content">
          <dl className="sysmon-summary">
            <div><dt>CPU</dt><dd>{snapshot ? `${snapshot.cpuPercent.toFixed(1)}%` : "-"}</dd></div>
            <div>
              <dt>{t("memory")}</dt>
              <dd>{snapshot ? `${snapshot.memoryPercent.toFixed(1)}%` : "-"}</dd>
              <small>{snapshot?.memoryTotalBytes ? `${formatBytes(memoryUsed)} / ${formatBytes(snapshot.memoryTotalBytes)}` : "-"}</small>
            </div>
            <div><dt>{t("load")}</dt><dd>{snapshot ? loadAverage.map((value) => value.toFixed(2)).join(" · ") : "-"}</dd></div>
            <div><dt>{t("receive")}</dt><dd>{snapshot ? `${snapshot.rxKbps.toFixed(1)} KiB/s` : "-"}</dd></div>
            <div><dt>{t("send")}</dt><dd>{snapshot ? `${snapshot.txKbps.toFixed(1)} KiB/s` : "-"}</dd></div>
            <div><dt>{t("uptime")}</dt><dd>{snapshot ? formatSysmonUptime(snapshot.uptimeSeconds) : "-"}</dd></div>
          </dl>

          <nav className="sysmon-tabs" aria-label={t("sysmon-details")}>
            <button className={tab === "processes" ? "active" : ""} onClick={() => setTab("processes")}>{t("processes")}<span>{processes.length}</span></button>
            <button className={tab === "disks" ? "active" : ""} onClick={() => setTab("disks")}>{t("disks")}<span>{disks.length}</span></button>
            <button className={tab === "network" ? "active" : ""} onClick={() => setTab("network")}>{t("network")}<span>{interfaces.length}</span></button>
            <button className={tab === "trends" ? "active" : ""} onClick={() => setTab("trends")}>{t("trends")}<span>{history.length}</span></button>
          </nav>

          <div className="sysmon-table-wrap">
            {tab === "trends" ? (
              <SysmonTrendView history={history} mode={trendMode} onModeChange={setTrendMode} error={historyError} loading={historyBusy} />
            ) : null}
            {tab === "processes" ? (
              <table className="sysmon-table sysmon-process-table">
                <thead><tr><th>PID</th><th>{t("processes")}</th><th>CPU</th><th>{t("memory")}</th><th>RSS</th></tr></thead>
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
                <thead><tr><th>{t("mount-point")}</th><th>{t("filesystem")}</th><th>{t("usage")}</th><th>{t("available")}</th><th>{t("total")}</th></tr></thead>
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
                <thead><tr><th>{t("interface")}</th><th>{t("ip-address")}</th><th>{t("receive-rate")}</th><th>{t("send-rate")}</th><th>{t("received")}</th><th>{t("sent-2")}</th></tr></thead>
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
              <div className="sysmon-empty">{t("the-current-sample-has-no-available")}{tab === "processes" ? t("processes") : tab === "disks" ? t("disks") : t("network-interfaces")}{t("details")}</div>
            ) : null}
            {!snapshot && canSample && !error && tab !== "trends" ? <div className="sysmon-empty loading"><LoaderCircle size={18} />{t("sampling")}</div> : null}
            {!snapshot && !canSample && tab !== "trends" ? <div className="sysmon-empty">{t("remote-session-disconnected")}</div> : null}
          </div>
          {error ? <div className="utility-error">{localizeDiagnostic(error)}</div> : null}
        </div>
        <footer className="sysmon-actions">
          <span>{snapshot ? t("sampled-at", [formatDateTime(snapshot.ts)]) : scope}</span>
          <button type="button" onClick={() => void refreshSysmonLive(session.profile.id)} disabled={busy || !canSample}>
            <RefreshCw size={14} className={busy ? "sysmon-refresh-icon loading" : "sysmon-refresh-icon"} />{t("refresh")}</button>
          <button type="button" onClick={onClose}>{t("close")}</button>
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
  loading,
}: {
  history: SysmonSnapshot[];
  mode: SysmonTrendMode;
  onModeChange: (mode: SysmonTrendMode) => void;
  error: string;
  loading: boolean;
}) {
  useLocale();
  const latest = history[history.length - 1];
  const first = history[0];
  const usageMode = mode === "usage";

  return (
    <section className="sysmon-trend-view">
      <header className="sysmon-trend-toolbar">
        <div className="sysmon-trend-modes" role="group" aria-label={t("trend-metric")}>
          <button type="button" className={usageMode ? "active" : ""} onClick={() => onModeChange("usage")}>{t("utilization")}</button>
          <button type="button" className={!usageMode ? "active" : ""} onClick={() => onModeChange("network")}>{t("network")}</button>
        </div>
        <div className="sysmon-trend-legend">
          <span className={usageMode ? "cpu" : "rx"}>{usageMode ? "CPU" : "RX"} <b>{latest ? formatSysmonTrendValue(latest, mode, 0) : "-"}</b></span>
          <span className={usageMode ? "memory" : "tx"}>{usageMode ? t("memory") : "TX"} <b>{latest ? formatSysmonTrendValue(latest, mode, 1) : "-"}</b></span>
        </div>
      </header>
      <div className="sysmon-trend-stage">
        <SysmonTrendCanvas history={history} mode={mode} />
        {!history.length ? <div className="sysmon-trend-empty">{loading ? t("loading-historical-samples") : t("no-historical-samples")}</div> : null}
      </div>
      <footer className="sysmon-trend-range">
        <span>{first ? formatEventClock(first.ts) : "--:--:--"}</span>
        <b>{t("samples", [history.length])}</b>
        <span>{latest ? formatEventClock(latest.ts) : "--:--:--"}</span>
      </footer>
      {error ? <div className="utility-error">{localizeDiagnostic(error)}</div> : null}
    </section>
  );
}

function SysmonTrendCanvas({ history, mode }: { history: SysmonSnapshot[]; mode: SysmonTrendMode }) {
  useLocale();
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
      context.font = '10px "JetBrains Mono", "Noto Sans Mono CJK SC", monospace';
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
      aria-label={t("trend-samples", [mode === "usage" ? t("cpu-and-memory-utilization") : t("network-receive-and-send-rates"), history.length])}
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
