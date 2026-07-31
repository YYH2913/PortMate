export const COMPLETED_TRANSFER_AUTO_DISMISS_MS = 6_000;
export const MAX_DISMISSED_TRANSFER_IDS = 10_000;

export function addDismissedTransferId(
  current: ReadonlySet<string>,
  transferId: string,
  limit = MAX_DISMISSED_TRANSFER_IDS,
): ReadonlySet<string> {
  if (current.has(transferId)) return current;
  const next = new Set(current);
  next.add(transferId);
  while (next.size > Math.max(1, limit)) {
    const oldest = next.values().next().value;
    if (oldest === undefined) break;
    next.delete(oldest);
  }
  return next;
}

export function completedTransferDismissDeadline(
  finishedAt: string | null | undefined,
  observedAt = Date.now(),
) {
  const parsed = finishedAt ? Date.parse(finishedAt) : Number.NaN;
  const completedAt = Number.isFinite(parsed) ? Math.min(parsed, observedAt) : observedAt;
  return completedAt + COMPLETED_TRANSFER_AUTO_DISMISS_MS;
}
