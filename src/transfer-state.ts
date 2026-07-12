import type { TransferTask } from "./types";

const terminalTransferStatuses = new Set<TransferTask["status"]>(["completed", "failed", "cancelled"]);

function newerTransferSnapshot(current: TransferTask, saved: TransferTask) {
  if (terminalTransferStatuses.has(current.status)) return current;
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
