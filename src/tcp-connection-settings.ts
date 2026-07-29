import type { TcpConnection } from "./types";

export const tcpConnectionDefaults = {
  reconnectDelayMs: 1_000,
  keepaliveEnabled: true,
  keepaliveIdleSeconds: 30,
  keepaliveIntervalSeconds: 10,
  keepaliveRetries: 3,
  telnetBinary: true,
  telnetNaws: true,
  tlsEnabled: false,
  tlsServerName: null,
  tlsAcceptInvalidCert: false,
} as const;

export const tcpConnectionBounds = {
  reconnectDelayMs: { min: 100, max: 60_000 },
  keepaliveIdleSeconds: { min: 1, max: 86_400 },
  keepaliveIntervalSeconds: { min: 1, max: 3_600 },
  keepaliveRetries: { min: 1, max: 20 },
} as const;

export function formatTcpConnectionTarget(
  kind: "telnet" | "tcp",
  connection: Pick<TcpConnection, "host" | "port" | "tlsEnabled">,
): string {
  const host = connection.host.trim();
  if (!host) return "";
  const scheme = kind === "telnet"
    ? (connection.tlsEnabled ? "telnets" : "telnet")
    : (connection.tlsEnabled ? "tcps" : "tcp");
  return `${scheme}://${host}:${connection.port}`;
}

function boundedInteger(value: unknown, fallback: number, min: number, max: number): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  return Math.min(max, Math.max(min, Math.trunc(value)));
}

function normalizeTlsServerName(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const normalized = value.trim();
  if (!normalized) return null;
  let characters = 0;
  for (const character of normalized) {
    characters += 1;
    if (characters > 253 || /[\s\u0000-\u001f\u007f]/u.test(character)) return null;
  }
  return normalized;
}

export function normalizeTcpConnectionSettings<T extends TcpConnection>(connection: T): T {
  return {
    ...connection,
    reconnect: typeof connection.reconnect === "boolean" ? connection.reconnect : true,
    telnetBinary: typeof connection.telnetBinary === "boolean" ? connection.telnetBinary : true,
    telnetNaws: typeof connection.telnetNaws === "boolean" ? connection.telnetNaws : true,
    tlsEnabled: typeof connection.tlsEnabled === "boolean" ? connection.tlsEnabled : false,
    tlsServerName: normalizeTlsServerName(connection.tlsServerName),
    tlsAcceptInvalidCert: typeof connection.tlsAcceptInvalidCert === "boolean"
      ? connection.tlsAcceptInvalidCert
      : false,
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
