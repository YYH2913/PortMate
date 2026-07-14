import type { SshConnection } from "./types";

export const sshConnectionDefaults = {
  reconnectDelayMs: 1_000,
  keepaliveEnabled: true,
  keepaliveIntervalSeconds: 30,
  keepaliveMaxMissed: 3,
} as const;

export const sshConnectionBounds = {
  reconnectDelayMs: { min: 100, max: 60_000 },
  keepaliveIntervalSeconds: { min: 1, max: 3_600 },
  keepaliveMaxMissed: { min: 1, max: 20 },
} as const;

function boundedInteger(value: unknown, fallback: number, min: number, max: number): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  return Math.min(max, Math.max(min, Math.trunc(value)));
}

export function normalizeSshConnectionSettings<T extends SshConnection>(connection: T): T {
  return {
    ...connection,
    reconnect: typeof connection.reconnect === "boolean" ? connection.reconnect : true,
    reconnectDelayMs: boundedInteger(
      connection.reconnectDelayMs,
      sshConnectionDefaults.reconnectDelayMs,
      sshConnectionBounds.reconnectDelayMs.min,
      sshConnectionBounds.reconnectDelayMs.max,
    ),
    keepaliveEnabled: typeof connection.keepaliveEnabled === "boolean" ? connection.keepaliveEnabled : true,
    keepaliveIntervalSeconds: boundedInteger(
      connection.keepaliveIntervalSeconds,
      sshConnectionDefaults.keepaliveIntervalSeconds,
      sshConnectionBounds.keepaliveIntervalSeconds.min,
      sshConnectionBounds.keepaliveIntervalSeconds.max,
    ),
    keepaliveMaxMissed: boundedInteger(
      connection.keepaliveMaxMissed,
      sshConnectionDefaults.keepaliveMaxMissed,
      sshConnectionBounds.keepaliveMaxMissed.min,
      sshConnectionBounds.keepaliveMaxMissed.max,
    ),
  } as T;
}
