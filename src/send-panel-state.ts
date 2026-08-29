export const DEFAULT_SEND_COUNT = 1;
export const DEFAULT_SEND_INTERVAL_MS = 1_000;
export const MAX_SEND_COUNT = 10_000;
export const MAX_SEND_INTERVAL_MS = 24 * 60 * 60 * 1_000;

export function normalizeSendCount(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return DEFAULT_SEND_COUNT;
  return Math.min(MAX_SEND_COUNT, Math.max(DEFAULT_SEND_COUNT, Math.trunc(value)));
}

export function normalizeSendInterval(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return DEFAULT_SEND_INTERVAL_MS;
  return Math.min(MAX_SEND_INTERVAL_MS, Math.max(0, Math.trunc(value)));
}

export type SendPanelWait = (milliseconds: number, signal?: AbortSignal) => Promise<void>;
export type SendPanelNow = () => number;
export type SendPanelDispatchOptions = { signal?: AbortSignal };

/**
 * Run repeated sender batches with a minimum start-to-start interval.
 *
 * Measuring from the start of each batch keeps the configured interval tied
 * to the wire scheduler rather than adding the transport write duration on
 * top of it. A slow write naturally pushes the next batch out, while a fast
 * write waits only for the remaining interval.
 */
export async function dispatchPacedSends(
  count: unknown,
  intervalMs: unknown,
  send: (index: number) => Promise<void>,
  wait: SendPanelWait = defaultSendPanelWait,
  now: SendPanelNow = defaultSendPanelNow,
  options: SendPanelDispatchOptions = {},
): Promise<void> {
  const total = normalizeSendCount(count);
  const interval = normalizeSendInterval(intervalMs);
  let lastStartedAt: number | null = null;
  for (let index = 0; index < total; index += 1) {
    throwIfSendPanelCancelled(options.signal);
    if (lastStartedAt !== null && interval > 0) {
      const remaining = lastStartedAt + interval - now();
      if (remaining > 0) await wait(Math.ceil(remaining), options.signal);
    }
    throwIfSendPanelCancelled(options.signal);
    lastStartedAt = now();
    await send(index);
  }
}

function defaultSendPanelWait(milliseconds: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const handleAbort = () => {
      clearTimeout(timer);
      reject(sendPanelCancelledError());
    };
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", handleAbort);
      resolve();
    }, milliseconds);
    if (signal?.aborted) handleAbort();
    else signal?.addEventListener("abort", handleAbort, { once: true });
  });
}

function defaultSendPanelNow(): number {
  return typeof performance !== "undefined" ? performance.now() : Date.now();
}

function throwIfSendPanelCancelled(signal?: AbortSignal) {
  if (signal?.aborted) throw sendPanelCancelledError();
}

function sendPanelCancelledError(): Error {
  const error = new Error("send operation cancelled");
  error.name = "AbortError";
  return error;
}
