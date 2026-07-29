import { describe, expect, it } from "vitest";
import type { TcpConnection } from "./types";
import { proxyDefaults } from "./proxy-settings";
import {
  formatTcpConnectionTarget,
  normalizeTcpConnectionSettings,
  tcpConnectionBounds,
  tcpConnectionDefaults,
} from "./tcp-connection-settings";

describe("TCP connection settings", () => {
  it("formats plain and TLS targets for TCP and Telnet", () => {
    const connection = { host: " console.example ", port: 23, tlsEnabled: false };
    expect(formatTcpConnectionTarget("tcp", connection)).toBe("tcp://console.example:23");
    expect(formatTcpConnectionTarget("telnet", connection)).toBe("telnet://console.example:23");
    expect(formatTcpConnectionTarget("tcp", { ...connection, tlsEnabled: true })).toBe("tcps://console.example:23");
    expect(formatTcpConnectionTarget("telnet", { ...connection, tlsEnabled: true })).toBe("telnets://console.example:23");
    expect(formatTcpConnectionTarget("tcp", { ...connection, host: "   " })).toBe("");
  });

  it("fills health defaults for legacy profiles", () => {
    const legacy = {
      host: "console.example",
      port: 23,
      reconnect: true,
      proxy: proxyDefaults,
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
      proxy: proxyDefaults,
      reconnectDelayMs: -4,
      keepaliveEnabled: true,
      keepaliveIdleSeconds: Number.MAX_SAFE_INTEGER,
      keepaliveIntervalSeconds: 4.9,
      keepaliveRetries: 999,
      telnetBinary: true,
      telnetNaws: true,
      tlsEnabled: false,
      tlsServerName: null,
      tlsAcceptInvalidCert: false,
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
      proxy: proxyDefaults,
      reconnectDelayMs: 2_500,
      keepaliveEnabled: false,
      keepaliveIdleSeconds: 90,
      keepaliveIntervalSeconds: 15,
      keepaliveRetries: 6,
      telnetBinary: false,
      telnetNaws: false,
      tlsEnabled: false,
      tlsServerName: null,
      tlsAcceptInvalidCert: false,
    });

    expect(normalized).toMatchObject({
      reconnect: false,
      reconnectDelayMs: 2_500,
      keepaliveEnabled: false,
      keepaliveIdleSeconds: 90,
      keepaliveIntervalSeconds: 15,
      keepaliveRetries: 6,
      telnetBinary: false,
      telnetNaws: false,
    });
  });

  it("normalizes TCP and Telnet TLS settings without enabling them for legacy profiles", () => {
    const normalized = normalizeTcpConnectionSettings({
      host: "console.example",
      port: 992,
      reconnect: true,
      proxy: proxyDefaults,
      ...tcpConnectionDefaults,
      tlsEnabled: true,
      tlsServerName: "  tls.console.example  ",
      tlsAcceptInvalidCert: true,
    });
    expect(normalized).toMatchObject({
      tlsEnabled: true,
      tlsServerName: "tls.console.example",
      tlsAcceptInvalidCert: true,
    });

    expect(normalizeTcpConnectionSettings({
      ...normalized,
      tlsServerName: "invalid server name",
    }).tlsServerName).toBeNull();
  });
});
