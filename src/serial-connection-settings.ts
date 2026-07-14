import type { SerialConnection } from "./types";

export const serialConnectionDefaults = {
  reconnectDelayMs: 1_000,
  receiveIdleTimeoutEnabled: false,
  receiveIdleTimeoutSeconds: 60,
} as const;

export const serialConnectionBounds = {
  reconnectDelayMs: { min: 100, max: 60_000 },
  receiveIdleTimeoutSeconds: { min: 1, max: 86_400 },
} as const;

function boundedInteger(value: unknown, fallback: number, min: number, max: number): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  return Math.min(max, Math.max(min, Math.trunc(value)));
}

export function normalizeSerialConnectionSettings<T extends SerialConnection>(connection: T): T {
  return {
    ...connection,
    reconnect: typeof connection.reconnect === "boolean" ? connection.reconnect : true,
    reconnectDelayMs: boundedInteger(
      connection.reconnectDelayMs,
      serialConnectionDefaults.reconnectDelayMs,
      serialConnectionBounds.reconnectDelayMs.min,
      serialConnectionBounds.reconnectDelayMs.max,
    ),
    receiveIdleTimeoutEnabled: typeof connection.receiveIdleTimeoutEnabled === "boolean"
      ? connection.receiveIdleTimeoutEnabled
      : false,
    receiveIdleTimeoutSeconds: boundedInteger(
      connection.receiveIdleTimeoutSeconds,
      serialConnectionDefaults.receiveIdleTimeoutSeconds,
      serialConnectionBounds.receiveIdleTimeoutSeconds.min,
      serialConnectionBounds.receiveIdleTimeoutSeconds.max,
    ),
  } as T;
}
