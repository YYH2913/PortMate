import { t } from "./i18n";
import type { SessionSummary } from "./types";

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
 * Wait a full interval after the preceding batch's transport acknowledgement.
 * Queue latency must never consume the interval and cause catch-up bursts.
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
  let lastCompletedAt: number | null = null;
  for (let index = 0; index < total; index += 1) {
    throwIfSendPanelCancelled(options.signal);
    if (lastCompletedAt !== null && interval > 0) {
      let remaining = lastCompletedAt + interval - now();
      while (remaining > 0) {
        await wait(Math.ceil(remaining), options.signal);
        throwIfSendPanelCancelled(options.signal);
        remaining = lastCompletedAt + interval - now();
      }
    }
    throwIfSendPanelCancelled(options.signal);
    await send(index);
    lastCompletedAt = now();
  }
}

export function parseHexBytes(value: string): number[] {
  if (!value.trim()) return [];
  const tokens = value.trim().split(/[\s,]+/);
  const bytes: number[] = [];
  for (const token of tokens) {
    const hex = token.replace(/^0x/i, "");
    if (!/^(?:[0-9a-f]{2})+$/i.test(hex)) {
      throw new Error(t("invalid-hex-each-byte-requires-two-hexadecimal-digits-optionally"));
    }
    for (let index = 0; index < hex.length; index += 2) bytes.push(Number.parseInt(hex.slice(index, index + 2), 16));
  }
  return bytes;
}

export function resolveSendTargets(target: "active" | "panes" | "connected", activeId: string,
  sessions: readonly SessionSummary[], panes: readonly SessionSummary[]): string[] {
  const candidates = target === "panes" ? panes
    : target === "active" ? sessions.filter((session) => session.profile.id === activeId) : sessions;
  return [...new Set(candidates.filter((session) => session.runtime.status === "connected").map((session) => session.profile.id))];
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
