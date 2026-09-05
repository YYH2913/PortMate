import { describe, expect, it } from "vitest";
import {
  changedAlternateTerminalRows,
  formatTerminalTimestampClock,
  normalizeTerminalTimestamps,
  rebaseTerminalTimestamps,
  resizeAlternateTerminalTimestamps,
  updateAlternateTerminalTimestamps,
  visibleSortedTerminalTimestamps,
  visibleTerminalTimestamps,
} from "./terminal-timestamp-state";

describe("terminal timestamp state", () => {
  it("queries only visible rows and a binary search prefix in a large live marker history", () => {
    const entries = Array.from({ length: 200_000 }, (_, line) => ({
      marker: { line },
      ts: line % 2 ? "2026-08-09T01:02:03.000000Z" : "2026-08-09T01:02:04.000000Z",
    }));
    let positionsRead = 0;
    let timestampsRead = 0;
    const visible = visibleSortedTerminalTimestamps(entries, 199_950, 40, (entry) => {
      positionsRead += 1;
      return entry.marker.line;
    }, (entry) => {
      timestampsRead += 1;
      return entry.ts;
    });
    expect(visible).toEqual(entries.slice(199_950, 199_990).map((entry, row) => ({
      line: entry.marker.line, ts: entry.ts, row,
    })));
    expect(positionsRead).toBeLessThan(64);
    expect(timestampsRead).toBe(40);
  });

  it("keeps inherited timestamps and suppresses repeated intervals in a sorted live range", () => {
    const entries = normalizeTerminalTimestamps([
      { line: 3, ts: "2026-08-09T01:02:03Z" },
      { line: 5, ts: "2026-08-09T01:02:03Z" },
      { line: 8, ts: "2026-08-09T01:02:04Z" },
      { line: 13, ts: "2026-08-09T01:02:05Z" },
    ]);
    for (let start = 0; start < 16; start += 1) {
      expect(visibleSortedTerminalTimestamps(entries, start, 4, (entry) => entry.line, (entry) => entry.ts))
        .toEqual(visibleTerminalTimestamps(entries, start, 4));
    }
    expect(visibleSortedTerminalTimestamps(entries, 0, 6, (entry) => entry.line, (entry) => entry.ts, entries[0].ts))
      .toEqual([{ line: 0, row: 0, ts: entries[0].ts }]);
    expect(visibleSortedTerminalTimestamps([], 9, 2, () => 0, () => "", entries[0].ts))
      .toEqual([{ line: 9, row: 0, ts: entries[0].ts }]);
  });

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

  it("shows the active interval once and labels timestamp transitions", () => {
    expect(visibleTerminalTimestamps([
      { line: 8, ts: "2026-08-09T01:02:01Z" },
      { line: 10, ts: "2026-08-09T01:02:02Z" },
      { line: 10, ts: "2026-08-09T01:02:03Z" },
      { line: 12, ts: "2026-08-09T01:02:04Z" },
      { line: 13, ts: "2026-08-09T01:02:05Z" },
    ], 10, 3)).toEqual([
      { line: 10, row: 0, ts: "2026-08-09T01:02:02.000000Z" },
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
    ]);
    expect(visibleTerminalTimestamps([
      { line: 12, ts: "2026-08-09T01:02:04.123456Z" },
    ], 13, 3)).toEqual([
      { line: 13, row: 0, ts: "2026-08-09T01:02:04.123456Z" },
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

  it("keeps dense per-row alternate timestamps and preserves unchanged rows", () => {
    const entered = resizeAlternateTerminalTimestamps([], 4, "2026-08-09T01:02:01.111111789Z");
    expect(entered).toEqual([
      { line: 0, ts: "2026-08-09T01:02:01.111111Z" },
      { line: 1, ts: "2026-08-09T01:02:01.111111Z" },
      { line: 2, ts: "2026-08-09T01:02:01.111111Z" },
      { line: 3, ts: "2026-08-09T01:02:01.111111Z" },
    ]);
    expect(updateAlternateTerminalTimestamps(
      entered,
      4,
      [1, 3],
      "2026-08-09T01:02:02.222222999Z",
    )).toEqual([
      { line: 0, ts: "2026-08-09T01:02:01.111111Z" },
      { line: 1, ts: "2026-08-09T01:02:02.222222Z" },
      { line: 2, ts: "2026-08-09T01:02:01.111111Z" },
      { line: 3, ts: "2026-08-09T01:02:02.222222Z" },
    ]);
  });

  it("fills added alternate rows with the latest event and crops removed rows", () => {
    const initial = [
      { line: 0, ts: "2026-08-09T01:02:01.111111Z" },
      { line: 1, ts: "2026-08-09T01:02:02.222222Z" },
    ];
    expect(resizeAlternateTerminalTimestamps(
      initial,
      4,
      "2026-08-09T01:02:03.333333Z",
    )).toEqual([
      ...initial,
      { line: 2, ts: "2026-08-09T01:02:03.333333Z" },
      { line: 3, ts: "2026-08-09T01:02:03.333333Z" },
    ]);
    expect(resizeAlternateTerminalTimestamps(initial, 1)).toEqual([initial[0]]);
    expect(resizeAlternateTerminalTimestamps(initial, 0)).toEqual([]);
  });

  it("detects changed alternate rows and always includes both cursor rows", () => {
    expect(changedAlternateTerminalRows(
      ["same", "old", "same", "removed"],
      ["same", "new", "same"],
      0,
      2,
    )).toEqual([0, 1, 2]);
  });
});
