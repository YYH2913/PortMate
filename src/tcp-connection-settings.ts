import type { TcpConnection } from "./types";

export const tcpConnectionDefaults = {
  reconnectDelayMs: 1_000,
  keepaliveEnabled: true,
  keepaliveIdleSeconds: 30,
  keepaliveIntervalSeconds: 10,
  keepaliveRetries: 3,
  telnetBinary: true,
  telnetNaws: true,
} as const;

export const tcpConnectionBounds = {
  reconnectDelayMs: { min: 100, max: 60_000 },
  keepaliveIdleSeconds: { min: 1, max: 86_400 },
  keepaliveIntervalSeconds: { min: 1, max: 3_600 },
  keepaliveRetries: { min: 1, max: 20 },
} as const;

function boundedInteger(value: unknown, fallback: number, min: number, max: number): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  return Math.min(max, Math.max(min, Math.trunc(value)));
}

export function normalizeTcpConnectionSettings<T extends TcpConnection>(connection: T): T {
  return {
    ...connection,
    reconnect: typeof connection.reconnect === "boolean" ? connection.reconnect : true,
    telnetBinary: typeof connection.telnetBinary === "boolean" ? connection.telnetBinary : true,
    telnetNaws: typeof connection.telnetNaws === "boolean" ? connection.telnetNaws : true,
    reconnectDelayMs: boundedInteger(
      connection.reconnectDelayMs,
      tcpConnectionDefaults.reconnectDelayMs,
      tcpConnectionBounds.reconnectDelayMs.min,
      tcpConnectionBounds.reconnectDelayMs.max,
    ),
    keepaliveEnabled: typeof connection.keepaliveEnabled === "boolean" ? connection.keepaliveEnabled : true,
    keepaliveIdleSeconds: boundedInteger(
      connection.keepaliveIdleSeconds,
      tcpConnectionDefaults.keepaliveIdleSeconds,
      tcpConnectionBounds.keepaliveIdleSeconds.min,
      tcpConnectionBounds.keepaliveIdleSeconds.max,
    ),
    keepaliveIntervalSeconds: boundedInteger(
      connection.keepaliveIntervalSeconds,
      tcpConnectionDefaults.keepaliveIntervalSeconds,
      tcpConnectionBounds.keepaliveIntervalSeconds.min,
      tcpConnectionBounds.keepaliveIntervalSeconds.max,
    ),
    keepaliveRetries: boundedInteger(
      connection.keepaliveRetries,
      tcpConnectionDefaults.keepaliveRetries,
      tcpConnectionBounds.keepaliveRetries.min,
      tcpConnectionBounds.keepaliveRetries.max,
    ),
  } as T;
}
