import { useEffect, useRef } from "react";
import { AlertCircle, Ban, CheckCircle2, Clock3, Copy, LoaderCircle, X } from "lucide-react";
import { formatBytes, formatDuration, formatEventClock } from "./display-formatters";
import { transferDiagnosticText, transferDisplayMessage, transferStatusLabel } from "./transfer-presentation";
import { completedTransferDismissDeadline } from "./transfer-visibility";
import type { TransferTask } from "./types";

export default function TransferList({
  transfers,
  dismissedTransferIds,
  busyTransferIds = EMPTY_BUSY_TRANSFER_IDS,
  onRetry,
  onCancel,
  onDismiss,
}: {
  transfers: readonly TransferTask[];
  dismissedTransferIds: ReadonlySet<string>;
  busyTransferIds?: ReadonlySet<string>;
  onRetry: (task: TransferTask) => void;
  onCancel: (task: TransferTask) => void;
  onDismiss: (transferId: string) => void;
}) {
  const dismissDeadlines = useRef(new Map<string, number>());

  useEffect(() => {
    const now = Date.now();
    const completedIds = new Set<string>();
    const timers = transfers
      .filter((task) => task.status === "completed")
      .map((task) => {
        completedIds.add(task.id);
        const completedDeadline = completedTransferDismissDeadline(task.finishedAt, now);
        const existingDeadline = dismissDeadlines.current.get(task.id);
        const deadline = existingDeadline === undefined
          ? completedDeadline
          : Math.min(existingDeadline, completedDeadline);
        dismissDeadlines.current.set(task.id, deadline);
        return window.setTimeout(() => {
          onDismiss(task.id);
        }, Math.max(0, deadline - Date.now()));
      });
    for (const id of dismissDeadlines.current.keys()) {
      if (!completedIds.has(id) && !transfers.some((task) => task.id === id && task.status === "completed")) {
        dismissDeadlines.current.delete(id);
      }
    }
    return () => timers.forEach((timer) => window.clearTimeout(timer));
  }, [onDismiss, transfers]);

  const visibleTransfers = transfers.filter((task) => !dismissedTransferIds.has(task.id));
  if (!visibleTransfers.length) return <div className="empty-pane top">没有传输任务</div>;
  return (
    <div className="transfer-list">
      {visibleTransfers.slice().reverse().map((task) => {
        const operationBusy = busyTransferIds.has(task.id);
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
                  <button type="button" disabled={operationBusy} onClick={() => onCancel(task)}>取消</button>
                ) : null}
                {task.status === "failed" || task.status === "cancelled" ? (
                  <button type="button" disabled={operationBusy} onClick={() => onRetry(task)}>重试</button>
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
                    onClick={() => onDismiss(task.id)}
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

const EMPTY_BUSY_TRANSFER_IDS: ReadonlySet<string> = new Set();
