import type { TransferTask } from "./types";

const terminalTransferStatuses = new Set<TransferTask["status"]>(["completed", "failed", "cancelled"]);

function mergeTerminalTransferSnapshot(current: TransferTask, saved: TransferTask) {
  if (saved.status !== current.status) return current;
  const next = {
    ...current,
    bytesTotal: Math.max(current.bytesTotal, saved.bytesTotal),
    bytesDone: Math.max(current.bytesDone, saved.bytesDone),
    message: current.message ?? saved.message,
    startedAt: current.startedAt ?? saved.startedAt,
    finishedAt: current.finishedAt ?? saved.finishedAt,
    averageBytesPerSecond: current.averageBytesPerSecond ?? saved.averageBytesPerSecond,
  };
  if (
    next.bytesTotal === current.bytesTotal
    && next.bytesDone === current.bytesDone
    && next.message === current.message
    && next.startedAt === current.startedAt
    && next.finishedAt === current.finishedAt
    && next.averageBytesPerSecond === current.averageBytesPerSecond
  ) return current;
  return next;
}

function newerTransferSnapshot(current: TransferTask, saved: TransferTask) {
  if (terminalTransferStatuses.has(current.status)) {
    return mergeTerminalTransferSnapshot(current, saved);
  }
  const currentRank = current.status === "queued" ? 0 : current.status === "running" ? 1 : 2;
  const savedRank = saved.status === "queued" ? 0 : saved.status === "running" ? 1 : 2;
  if (savedRank < currentRank) return current;
  if (savedRank > currentRank) return saved;
  if (saved.bytesDone < current.bytesDone) return current;
  if (saved.bytesDone === current.bytesDone) {
    if (current.bytesTotal > 0 && saved.bytesTotal === 0) return current;
    if (current.startedAt && !saved.startedAt) return current;
  }
  return saved;
}

export function mergeTransfers(current: TransferTask[], saved: TransferTask) {
  const index = current.findIndex((task) => task.id === saved.id);
  if (index < 0) return [...current, saved];
  return current.map((task, itemIndex) => itemIndex === index ? newerTransferSnapshot(task, saved) : task);
}
