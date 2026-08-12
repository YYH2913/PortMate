import { describe, expect, it } from "vitest";
import {
  formatTerminalTimestampClock,
  normalizeTerminalTimestamps,
  rebaseTerminalTimestamps,
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

  it("does not assign a future event timestamp to rows before the first marker", () => {
    expect(visibleTerminalTimestamps([
      { line: 12, ts: "2026-08-09T01:02:04.123456Z" },
    ], 10, 2)).toEqual([]);
    expect(visibleTerminalTimestamps([
      { line: 12, ts: "2026-08-09T01:02:04.123456Z" },
    ], 11, 3)).toEqual([
      { line: 12, row: 1, ts: "2026-08-09T01:02:04.123456Z" },
      { line: 13, row: 2, ts: "2026-08-09T01:02:04.123456Z" },
    ]);
  });

  it("preserves the active timestamp interval when a serialized window starts mid-range", () => {
    expect(rebaseTerminalTimestamps([
      { line: 3, ts: "2026-08-09T01:02:03.111111Z" },
      { line: 8, ts: "2026-08-09T01:02:04.222222Z" },
      { line: 14, ts: "2026-08-09T01:02:05.333333Z" },
    ], 10)).toEqual([
      { line: 0, ts: "2026-08-09T01:02:04.222222Z" },
      { line: 4, ts: "2026-08-09T01:02:05.333333Z" },
    ]);
  });

  it("preserves sub-millisecond precision and displays a fixed microsecond clock", () => {
    expect(normalizeTerminalTimestamps([
      { line: 0, ts: "2026-08-09T01:02:03.123456789Z" },
      { line: 1, ts: "2026-08-09T09:02:03.4+08:00" },
    ])).toEqual([
      { line: 0, ts: "2026-08-09T01:02:03.123456Z" },
      { line: 1, ts: "2026-08-09T01:02:03.400000Z" },
    ]);
    expect(formatTerminalTimestampClock("2026-08-09T01:02:03.123456789Z"))
      .toMatch(/^\d{2}:\d{2}:\d{2}\.123456$/);
    expect(formatTerminalTimestampClock("invalid")).toBe("--:--:--.------");
  });
});
