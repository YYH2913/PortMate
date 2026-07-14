import { describe, expect, it } from "vitest";
import { mergeSysmonHistory, normalizeSysmonHistory, sysmonTrendMax, sysmonTrendValue } from "./sysmon-history";
import type { SysmonSnapshot } from "./types";

function snapshot(sessionId: string, ts: string, cpuPercent: number, rxKbps = 0): SysmonSnapshot {
  return {
    sessionId,
    ts,
    uptimeSeconds: 1,
    cpuPercent,
    memoryPercent: 50,
    rxKbps,
    txKbps: rxKbps / 2,
    loadAverage: [0, 0, 0],
    memoryTotalBytes: 1,
    memoryAvailableBytes: 1,
    processes: [],
    disks: [],
    networkInterfaces: [],
  };
}

describe("Sysmon history", () => {
  it("filters sessions, rejects invalid timestamps, deduplicates, sorts, and limits", () => {
    const first = snapshot("target", "2026-07-14T10:00:00Z", 10);
    const replacement = snapshot("target", "2026-07-14T10:00:00Z", 20);
    const latest = snapshot("target", "2026-07-14T10:02:00Z", 30);
    const middle = snapshot("target", "2026-07-14T10:01:00Z", 25);
    const history = normalizeSysmonHistory([
      latest,
      first,
      snapshot("other", "2026-07-14T10:03:00Z", 99),
      snapshot("target", "invalid", 99),
      replacement,
      middle,
    ], "target", 2);

    expect(history.map((item) => item.ts)).toEqual([middle.ts, latest.ts]);
    expect(normalizeSysmonHistory([first, replacement], "target", 10)[0].cpuPercent).toBe(20);
  });

  it("merges replacement samples while keeping the newest bounded range", () => {
    const first = snapshot("target", "2026-07-14T10:00:00Z", 10);
    const second = snapshot("target", "2026-07-14T10:01:00Z", 20);
    const replacement = snapshot("target", second.ts, 80);
    const third = snapshot("target", "2026-07-14T10:02:00Z", 30);

    const history = mergeSysmonHistory(
      mergeSysmonHistory([first, second], replacement, 2),
      third,
      2,
    );
    expect(history.map((item) => item.cpuPercent)).toEqual([80, 30]);
  });

  it("uses fixed utilization and rounded network domains with bounded values", () => {
    const history = [
      snapshot("target", "2026-07-14T10:00:00Z", -5, 12),
      snapshot("target", "2026-07-14T10:01:00Z", 120, 32),
    ];
    expect(sysmonTrendMax(history, "usage")).toBe(100);
    expect(sysmonTrendMax(history, "network")).toBe(50);
    expect(sysmonTrendValue(history[0], "usage", 0)).toBe(0);
    expect(sysmonTrendValue(history[1], "usage", 0)).toBe(100);
  });
});
