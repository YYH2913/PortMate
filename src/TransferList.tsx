import { useEffect, useRef, useState } from "react";
import { AlertCircle, Ban, CheckCircle2, Clock3, Copy, LoaderCircle, X } from "lucide-react";
import { formatBytes, formatDuration, formatEventClock } from "./display-formatters";
import { transferDiagnosticText, transferDisplayMessage, transferStatusLabel } from "./transfer-presentation";
import type { TransferTask } from "./types";

const COMPLETED_TRANSFER_AUTO_DISMISS_MS = 6_000;

export default function TransferList({
  transfers,
  onRetry,
  onCancel,
}: {
  transfers: readonly TransferTask[];
  onRetry: (task: TransferTask) => void;
  onCancel: (task: TransferTask) => void;
}) {
  const [dismissed, setDismissed] = useState<Set<string>>(() => new Set());
  const dismissDeadlines = useRef(new Map<string, number>());

  useEffect(() => {
    const now = Date.now();
    const completedIds = new Set<string>();
    const timers = transfers
      .filter((task) => task.status === "completed")
      .map((task) => {
        completedIds.add(task.id);
        const deadline = dismissDeadlines.current.get(task.id) ?? now + COMPLETED_TRANSFER_AUTO_DISMISS_MS;
        dismissDeadlines.current.set(task.id, deadline);
        return window.setTimeout(() => {
          setDismissed((current) => {
            if (current.has(task.id)) return current;
            const next = new Set(current);
            next.add(task.id);
            return next;
          });
        }, Math.max(0, deadline - Date.now()));
      });
    for (const id of dismissDeadlines.current.keys()) {
      if (!completedIds.has(id) && !transfers.some((task) => task.id === id && task.status === "completed")) {
        dismissDeadlines.current.delete(id);
      }
    }
    return () => timers.forEach((timer) => window.clearTimeout(timer));
  }, [transfers]);

  const visibleTransfers = transfers.filter((task) => !dismissed.has(task.id));
  if (!visibleTransfers.length) return <div className="empty-pane top">没有传输任务</div>;
  return (
    <div className="transfer-list">
      {visibleTransfers.slice().reverse().map((task) => {
        const message = transferDisplayMessage(task);
        const StatusIcon = task.status === "queued" ? Clock3
          : task.status === "running" ? LoaderCircle
            : task.status === "completed" ? CheckCircle2
              : task.status === "cancelled" ? Ban
                : AlertCircle;
        return (
          <div key={task.id} className={`transfer-row status-${task.status}`}>
            <div className="transfer-row-head">
              <strong>{task.protocol}</strong>
              <span className="transfer-status"><StatusIcon size={14} /><span>{transferStatusLabel(task.status)}</span></span>
              <div className="transfer-row-actions">
                {task.status === "running" ? (
                  <button type="button" onClick={() => onCancel(task)}>取消</button>
                ) : null}
                {task.status === "failed" || task.status === "cancelled" ? (
                  <button type="button" onClick={() => onRetry(task)}>重试</button>
                ) : null}
                {task.status === "failed" ? (
                  <button
                    className="transfer-icon-button"
                    type="button"
                    title="复制失败诊断"
                    aria-label="复制失败诊断"
                    onClick={() => void navigator.clipboard?.writeText(transferDiagnosticText(task)).catch(() => {})}
                  >
                    <Copy size={14} />
                  </button>
                ) : null}
                {task.status === "completed" || task.status === "failed" || task.status === "cancelled" ? (
                  <button
                    className="transfer-icon-button"
                    type="button"
                    title={task.status === "completed" ? "关闭已完成传输" : "隐藏传输记录"}
                    aria-label={task.status === "completed" ? "关闭已完成传输" : "隐藏传输记录"}
                    onClick={() => setDismissed((current) => new Set(current).add(task.id))}
                  >
                    <X size={14} />
                  </button>
                ) : null}
              </div>
            </div>
            <small title={`${task.source} → ${task.destination}`}>{task.source} → {task.destination}</small>
            <small>
              {formatBytes(task.bytesDone)} / {task.bytesTotal ? formatBytes(task.bytesTotal) : "未知"}
              {task.averageBytesPerSecond ? ` · ${formatBytes(task.averageBytesPerSecond)}/s` : ""}
              {task.startedAt && task.finishedAt ? ` · ${formatDuration(task.startedAt, task.finishedAt)}` : ""}
              {task.status === "failed" && task.finishedAt ? ` · ${formatEventClock(task.finishedAt)}` : ""}
            </small>
            {message ? <div className="transfer-message" role={task.status === "failed" ? "alert" : undefined} title={message}><AlertCircle size={14} /><span>{message}</span></div> : null}
            <div className="transfer-progress">
              <span style={{ width: `${task.bytesTotal ? Math.min(100, (task.bytesDone / task.bytesTotal) * 100) : task.status === "completed" ? 100 : 0}%` }} />
            </div>
          </div>
        );
      })}
    </div>
  );
}
