import { useState } from "react";
import { Activity, LoaderCircle, Maximize2, RefreshCw } from "lucide-react";
import { formatBytes, formatEventClock } from "./display-formatters";
import { refreshSysmonLive, useSysmonLivePolling, useSysmonLiveState } from "./sysmon-live-state";
import { formatSysmonNetworkAddresses } from "./sysmon-network-addresses";
import type { SessionSummary } from "./types";

type SysmonSidebarTab = "processes" | "disks" | "network";

export default function SysmonSidebar({
  session,
  enabled,
  onOpenDetails,
}: {
  session: SessionSummary | null;
  enabled: boolean;
  onOpenDetails: () => void;
}) {
  const [tab, setTab] = useState<SysmonSidebarTab>("processes");
  const remote = session ? isSshLikeSession(session) : false;
  const canSample = Boolean(session) && (!remote || session?.runtime.status === "connected");
  const live = useSysmonLiveState(session?.profile.id);
  useSysmonLivePolling(session?.profile.id, enabled && canSample);
  const snapshot = live.snapshot;
  const busy = enabled && canSample && live.busy;
  const error = live.error;

  if (!session) {
    return <div className="workspace-sysmon-empty"><Activity size={18} /><span>选择会话后开始监控</span></div>;
  }

  const processes = snapshot?.processes ?? [];
  const disks = snapshot?.disks ?? [];
  const interfaces = snapshot?.networkInterfaces ?? [];

  return (
    <section className="workspace-sysmon" aria-label={`${session.profile.name} Sysmon`}>
      <header className="workspace-sysmon-header">
        <div>
          <strong title={session.profile.name}>{session.profile.name}</strong>
          <span>{snapshot ? formatEventClock(snapshot.ts) : remote ? "远端主机" : "本机"}</span>
        </div>
        <button type="button" title="刷新 Sysmon" aria-label="刷新 Sysmon" onClick={() => void refreshSysmonLive(session.profile.id)} disabled={!canSample || busy}>
          <RefreshCw size={14} className={busy ? "loading" : ""} />
        </button>
        <button type="button" title="打开 Sysmon 详情" aria-label="打开 Sysmon 详情" onClick={onOpenDetails}>
          <Maximize2 size={14} />
        </button>
      </header>

      <div className="workspace-sysmon-notices">
        {!canSample ? <div className="workspace-sysmon-message">远端会话未连接</div> : null}
        {error ? <div className="workspace-sysmon-message error" title={error}>{error}</div> : null}
      </div>

      <dl className="workspace-sysmon-summary">
        <SysmonMetric label="CPU" value={snapshot ? `${snapshot.cpuPercent.toFixed(1)}%` : "-"} percent={snapshot?.cpuPercent} />
        <SysmonMetric label="内存" value={snapshot ? `${snapshot.memoryPercent.toFixed(1)}%` : "-"} percent={snapshot?.memoryPercent} />
        <div><dt>RX</dt><dd>{snapshot ? formatRate(snapshot.rxKbps) : "-"}</dd></div>
        <div><dt>TX</dt><dd>{snapshot ? formatRate(snapshot.txKbps) : "-"}</dd></div>
      </dl>

      <nav className="workspace-sysmon-tabs" aria-label="Sysmon 侧栏详情">
        <button type="button" className={tab === "processes" ? "active" : ""} onClick={() => setTab("processes")}>进程 <span>{processes.length}</span></button>
        <button type="button" className={tab === "disks" ? "active" : ""} onClick={() => setTab("disks")}>磁盘 <span>{disks.length}</span></button>
        <button type="button" className={tab === "network" ? "active" : ""} onClick={() => setTab("network")}>网络 <span>{interfaces.length}</span></button>
      </nav>

      <div className="workspace-sysmon-list">
        {tab === "processes" ? processes.map((process) => (
          <div className="workspace-sysmon-row process" key={process.pid}>
            <span className="mono">{process.pid}</span>
            <strong title={process.name}>{process.name}</strong>
            <span>{process.cpuPercent.toFixed(1)}%</span>
            <small>{formatBytes(process.rssBytes)}</small>
          </div>
        )) : null}
        {tab === "disks" ? disks.map((disk) => (
          <div className="workspace-sysmon-row disk" key={`${disk.filesystem}-${disk.mountPoint}`}>
            <strong title={disk.mountPoint}>{disk.mountPoint}</strong>
            <span>{disk.usedPercent.toFixed(1)}%</span>
            <small>{formatBytes(disk.availableBytes)} 可用</small>
          </div>
        )) : null}
        {tab === "network" ? interfaces.map((item) => (
          <div className="workspace-sysmon-row network" key={item.name}>
            <strong title={item.name}>{item.name}</strong>
            <span title={item.addresses.join(" / ")}>{formatSysmonNetworkAddresses(item.addresses)}</span>
            <small>R {formatRate(item.rxKbps)} · T {formatRate(item.txKbps)}</small>
          </div>
        )) : null}
        {snapshot && ((tab === "processes" && !processes.length) || (tab === "disks" && !disks.length) || (tab === "network" && !interfaces.length))
          ? <div className="workspace-sysmon-list-empty">当前采样没有明细</div>
          : null}
        {!snapshot && canSample && !error ? <div className="workspace-sysmon-list-empty loading"><LoaderCircle size={16} />正在采样</div> : null}
      </div>
    </section>
  );
}

function SysmonMetric({ label, value, percent }: { label: string; value: string; percent?: number }) {
  const bounded = Math.min(100, Math.max(0, percent ?? 0));
  return (
    <div>
      <dt>{label}</dt>
      <dd>{value}</dd>
      <span><i style={{ width: `${bounded}%` }} /></span>
    </div>
  );
}

function isSshLikeSession(session: SessionSummary) {
  return session.profile.connection.kind === "ssh" || session.profile.connection.kind === "tmux";
}

function formatRate(kibibytesPerSecond: number) {
  if (kibibytesPerSecond >= 1024) return `${(kibibytesPerSecond / 1024).toFixed(1)}M`;
  return `${kibibytesPerSecond.toFixed(1)}K`;
}
