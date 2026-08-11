import { describe, expect, it } from "vitest";
import {
  formatTerminalTimestampClock,
  normalizeTerminalTimestamps,
  visibleTerminalTimestamps,
} from "./terminal-timestamp-state";

describe("terminal timestamp state", () => {
  it("normalizes timestamps, keeps the first event per line, and bounds old rows", () => {
    expect(normalizeTerminalTimestamps([
      { line: 3, ts: "2026-08-09T01:02:03Z" },
      { line: 1, ts: "2026-08-09T01:02:01Z" },
      { line: 3, ts: "2026-08-09T01:02:04Z" },
      { line: -1, ts: "2026-08-09T01:02:00Z" },
      { line: 4, ts: "invalid" },
    ], 2)).toEqual([
      { line: 1, ts: "2026-08-09T01:02:01.000000Z" },
      { line: 3, ts: "2026-08-09T01:02:03.000000Z" },
    ]);
  });

  it("fills every visible row from the nearest preceding timestamp", () => {
    expect(visibleTerminalTimestamps([
      { line: 8, ts: "2026-08-09T01:02:01Z" },
      { line: 10, ts: "2026-08-09T01:02:02Z" },
      { line: 10, ts: "2026-08-09T01:02:03Z" },
      { line: 12, ts: "2026-08-09T01:02:04Z" },
      { line: 13, ts: "2026-08-09T01:02:05Z" },
    ], 10, 3)).toEqual([
      { line: 10, row: 0, ts: "2026-08-09T01:02:02.000000Z" },
      { line: 11, row: 1, ts: "2026-08-09T01:02:02.000000Z" },
      { line: 12, row: 2, ts: "2026-08-09T01:02:04.000000Z" },
    ]);
  });

  it("preserves sub-millisecond precision and displays a fixed microsecond clock", () => {
    expect(normalizeTerminalTimestamps([
      { line: 0, ts: "2026-08-09T01:02:03.123456789Z" },
      { line: 1, ts: "2026-08-09T09:02:03.4+08:00" },
    ])).toEqual([
      { line: 0, ts: "2026-08-09T01:02:03.123456789Z" },
      { line: 1, ts: "2026-08-09T01:02:03.400000Z" },
    ]);
    expect(formatTerminalTimestampClock("2026-08-09T01:02:03.123456789Z"))
      .toMatch(/^\d{2}:\d{2}:\d{2}\.123456$/);
    expect(formatTerminalTimestampClock("invalid")).toBe("--:--:--.------");
  });
});
