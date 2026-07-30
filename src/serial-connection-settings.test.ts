import { describe, expect, it } from "vitest";
import {
  COMMON_SERIAL_BAUD_RATES,
  normalizeSerialConnectionSettings,
  serialConnectionBounds,
  serialConnectionDefaults,
} from "./serial-connection-settings";
import type { SerialConnection } from "./types";

function baseConnection(): SerialConnection {
  return {
    port: "/dev/ttyUSB0",
    baudRate: 115_200,
    dataBits: 8,
    stopBits: 1,
    parity: "none",
    flowControl: "none",
    dtr: false,
    rts: false,
    reconnect: true,
    ...serialConnectionDefaults,
  };
}

describe("serial connection settings", () => {
  it("offers common baud rates from legacy devices through high-speed adapters", () => {
    expect(COMMON_SERIAL_BAUD_RATES).toEqual([
      110,
      300,
      600,
      1_200,
      2_400,
      4_800,
      9_600,
      14_400,
      19_200,
      38_400,
      57_600,
      115_200,
      230_400,
      460_800,
      921_600,
      1_000_000,
      1_500_000,
    ]);
  });

  it("fills health defaults for legacy profiles", () => {
    const legacy = baseConnection() as Partial<SerialConnection>;
    delete legacy.reconnect;
    delete legacy.reconnectDelayMs;
    delete legacy.receiveIdleTimeoutEnabled;
    delete legacy.receiveIdleTimeoutSeconds;

    expect(normalizeSerialConnectionSettings(legacy as SerialConnection)).toMatchObject({
      reconnect: true,
      ...serialConnectionDefaults,
    });
  });

  it("clamps and truncates operational settings", () => {
    const normalized = normalizeSerialConnectionSettings({
      ...baseConnection(),
      reconnectDelayMs: -1,
      receiveIdleTimeoutSeconds: Number.MAX_SAFE_INTEGER,
    });

    expect(normalized.reconnectDelayMs).toBe(serialConnectionBounds.reconnectDelayMs.min);
    expect(normalized.receiveIdleTimeoutSeconds).toBe(serialConnectionBounds.receiveIdleTimeoutSeconds.max);
  });

  it("preserves disabled switches and valid custom values", () => {
    const normalized = normalizeSerialConnectionSettings({
      ...baseConnection(),
      reconnect: false,
      reconnectDelayMs: 2_500,
      receiveIdleTimeoutEnabled: true,
      receiveIdleTimeoutSeconds: 90,
    });

    expect(normalized).toMatchObject({
      reconnect: false,
      reconnectDelayMs: 2_500,
      receiveIdleTimeoutEnabled: true,
      receiveIdleTimeoutSeconds: 90,
    });
  });
});
