import { describe, expect, it } from "vitest";
import type { TcpConnection } from "./types";
import {
  normalizeTcpConnectionSettings,
  tcpConnectionBounds,
  tcpConnectionDefaults,
} from "./tcp-connection-settings";

describe("TCP connection settings", () => {
  it("fills health defaults for legacy profiles", () => {
    const legacy = {
      host: "console.example",
      port: 23,
      reconnect: true,
    } as TcpConnection;

    expect(normalizeTcpConnectionSettings(legacy)).toEqual({
      ...legacy,
      ...tcpConnectionDefaults,
    });
  });

  it("clamps and truncates operational settings", () => {
    const normalized = normalizeTcpConnectionSettings({
      host: "console.example",
      port: 23,
      reconnect: true,
      reconnectDelayMs: -4,
      keepaliveEnabled: true,
      keepaliveIdleSeconds: Number.MAX_SAFE_INTEGER,
      keepaliveIntervalSeconds: 4.9,
      keepaliveRetries: 999,
    });

    expect(normalized.reconnectDelayMs).toBe(tcpConnectionBounds.reconnectDelayMs.min);
    expect(normalized.keepaliveIdleSeconds).toBe(tcpConnectionBounds.keepaliveIdleSeconds.max);
    expect(normalized.keepaliveIntervalSeconds).toBe(4);
    expect(normalized.keepaliveRetries).toBe(tcpConnectionBounds.keepaliveRetries.max);
  });

  it("preserves explicit disabled switches and valid custom values", () => {
    const normalized = normalizeTcpConnectionSettings({
      host: "console.example",
      port: 2323,
      reconnect: false,
      reconnectDelayMs: 2_500,
      keepaliveEnabled: false,
      keepaliveIdleSeconds: 90,
      keepaliveIntervalSeconds: 15,
      keepaliveRetries: 6,
    });

    expect(normalized).toMatchObject({
      reconnect: false,
      reconnectDelayMs: 2_500,
      keepaliveEnabled: false,
      keepaliveIdleSeconds: 90,
      keepaliveIntervalSeconds: 15,
      keepaliveRetries: 6,
    });
  });
});
