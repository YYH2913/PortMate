import { describe, expect, it } from "vitest";
import {
  analyzeSerialCaptureFrames,
  defaultSerialAnalyzerStoredState,
  filterSerialAnalyzedFrames,
  MAX_SERIAL_ANALYZED_FRAMES,
  MAX_SERIAL_ANALYZER_BOOKMARKS,
  normalizeSerialAnalyzerStoredState,
  serialAnalyzerDelimiterBytes,
  serialAnalyzerHexDump,
  toggleSerialAnalyzerBookmark,
} from "./serial-analyzer-state";
import type { SerialCaptureFrame } from "./types";

const frame = (
  id: string,
  direction: SerialCaptureFrame["direction"],
  bytes: number[],
  ts = "2026-07-15T00:00:00.000Z",
  truncated = false,
): SerialCaptureFrame => ({ id, ts, direction, bytes, originalLength: bytes.length, truncated });

describe("serial analyzer state", () => {
  it("normalizes persisted parser settings, filters and bookmark bounds", () => {
    const stored = normalizeSerialAnalyzerStoredState({
      parser: { mode: "fixed", delimiterHex: "0x0d, 0x0a", includeDelimiter: false, fixedLength: 99_999, gapMs: 0 },
      direction: "outbound",
      pageSize: 500,
      follow: false,
      bookmarksOnly: true,
      bookmarks: { serial: Array.from({ length: MAX_SERIAL_ANALYZER_BOOKMARKS + 2 }, (_, index) => `frame-${index}`) },
    });
    expect(stored.parser).toMatchObject({ mode: "fixed", delimiterHex: "0D 0A", includeDelimiter: false, fixedLength: 4096, gapMs: 1 });
    expect(stored).toMatchObject({ direction: "outbound", pageSize: 500, follow: false, bookmarksOnly: true });
    expect(stored.bookmarks.serial).toHaveLength(MAX_SERIAL_ANALYZER_BOOKMARKS);
  });

  it("parses delimiters across capture chunks without crossing directions", () => {
    const result = analyzeSerialCaptureFrames([
      frame("rx-a", "inbound", [0x41, 0x0d]),
      frame("rx-b", "inbound", [0x0a, 0x42, 0x0d, 0x0a]),
      frame("tx-a", "outbound", [0x43]),
    ], { mode: "delimiter", delimiterHex: "0D 0A", includeDelimiter: false, fixedLength: 8, gapMs: 20 });
    expect(result.frames.map((item) => ({ direction: item.direction, bytes: item.bytes, complete: item.complete, sources: item.sourceFrameIds }))).toEqual([
      { direction: "inbound", bytes: [0x41], complete: true, sources: ["rx-a", "rx-b"] },
      { direction: "inbound", bytes: [0x42], complete: true, sources: ["rx-b"] },
      { direction: "outbound", bytes: [0x43], complete: false, sources: ["tx-a"] },
    ]);
  });

  it("builds fixed frames, preserves partial tails and truncation evidence", () => {
    const result = analyzeSerialCaptureFrames([
      frame("a", "inbound", [1, 2, 3], undefined, true),
      frame("b", "inbound", [4, 5]),
      frame("c", "outbound", [6, 7]),
    ], { mode: "fixed", delimiterHex: "0A", includeDelimiter: true, fixedLength: 4, gapMs: 20 });
    expect(result.frames.map((item) => [item.direction, item.bytes, item.complete, item.truncated])).toEqual([
      ["inbound", [1, 2, 3, 4], true, true],
      ["inbound", [5], false, false],
      ["outbound", [6, 7], false, false],
    ]);
  });

  it("groups same-direction chunks until an idle gap or direction boundary", () => {
    const result = analyzeSerialCaptureFrames([
      frame("a", "inbound", [1], "2026-07-15T00:00:00.000Z"),
      frame("b", "inbound", [2], "2026-07-15T00:00:00.010Z"),
      frame("c", "inbound", [3], "2026-07-15T00:00:00.050Z"),
      frame("d", "outbound", [4], "2026-07-15T00:00:00.055Z"),
    ], { mode: "gap", delimiterHex: "0A", includeDelimiter: true, fixedLength: 8, gapMs: 20 });
    expect(result.frames.map((item) => [item.direction, item.bytes, item.complete])).toEqual([
      ["inbound", [1, 2], true],
      ["inbound", [3], true],
      ["outbound", [4], false],
    ]);
  });

  it("bounds high-cardinality analysis with a ring instead of retaining every byte frame", () => {
    const bytes = Array.from({ length: MAX_SERIAL_ANALYZED_FRAMES + 20 }, (_, index) => index % 256);
    const result = analyzeSerialCaptureFrames([frame("large", "inbound", bytes)], {
      mode: "fixed", delimiterHex: "0A", includeDelimiter: true, fixedLength: 1, gapMs: 20,
    });
    expect(result.frames).toHaveLength(MAX_SERIAL_ANALYZED_FRAMES);
    expect(result.totalFrames).toBe(MAX_SERIAL_ANALYZED_FRAMES + 20);
    expect(result.droppedFrames).toBe(20);
    expect(result.frames[0].bytes).toEqual([20]);
  });

  it("filters analyzed bytes and toggles stable source bookmarks immutably", () => {
    const result = analyzeSerialCaptureFrames([
      frame("rx", "inbound", [0xff, 0x00]),
      frame("tx", "outbound", [0x48, 0x69]),
    ], { mode: "capture", delimiterHex: "0A", includeDelimiter: true, fixedLength: 8, gapMs: 20 });
    const marked = toggleSerialAnalyzerBookmark(defaultSerialAnalyzerStoredState, "serial", "rx");
    expect(marked).not.toBe(defaultSerialAnalyzerStoredState);
    expect(filterSerialAnalyzedFrames(result.frames, "inbound", "FF00", new Set(marked.bookmarks.serial), true)).toEqual([result.frames[0]]);
    expect(toggleSerialAnalyzerBookmark(marked, "serial", "rx").bookmarks.serial).toBeUndefined();
  });

  it("validates delimiter input and renders bounded dumps", () => {
    expect(serialAnalyzerDelimiterBytes("0xAA:55 0d0a")).toEqual([0xaa, 0x55, 0x0d, 0x0a]);
    expect(serialAnalyzerDelimiterBytes("ABC")).toBeNull();
    expect(serialAnalyzerHexDump([0x41, 0, 0x42], 16)).toContain("00000000  41 00 42");
  });
});
