import type { SshConnection } from "./types";

export const sshConnectionDefaults = {
  keepaliveEnabled: true,
  keepaliveIntervalSeconds: 30,
  keepaliveMaxMissed: 3,
} as const;

export const sshConnectionBounds = {
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
