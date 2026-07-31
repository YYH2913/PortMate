export const COMPLETED_TRANSFER_AUTO_DISMISS_MS = 6_000;

export function completedTransferDismissDeadline(
  finishedAt: string | null | undefined,
  observedAt = Date.now(),
) {
  const parsed = finishedAt ? Date.parse(finishedAt) : Number.NaN;
  const completedAt = Number.isFinite(parsed) ? Math.min(parsed, observedAt) : observedAt;
  return completedAt + COMPLETED_TRANSFER_AUTO_DISMISS_MS;
}
