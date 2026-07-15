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
  serialModbusSilenceMs,
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
      parser: { mode: "fixed", delimiterHex: "0x0d, 0x0a", includeDelimiter: false, fixedLength: 99_999, gapMs: 0, modbusAutoGap: false, modbusGapMs: 99_999 },
      direction: "outbound",
      pageSize: 500,
      follow: false,
      bookmarksOnly: true,
      bookmarks: { serial: Array.from({ length: MAX_SERIAL_ANALYZER_BOOKMARKS + 2 }, (_, index) => `frame-${index}`) },
    });
    expect(stored.parser).toMatchObject({ mode: "fixed", delimiterHex: "0D 0A", includeDelimiter: false, fixedLength: 4096, gapMs: 1, modbusAutoGap: false, modbusGapMs: 60_000 });
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

  it("decodes RFC 1055 SLIP frames across capture chunks and preserves wire evidence", () => {
    const result = analyzeSerialCaptureFrames([
      frame("rx-a", "inbound", [0xc0, 0x41, 0xdb]),
      frame("rx-b", "inbound", [0xdc, 0x42, 0xdb, 0xdd, 0xc0, 0xc0, 0x43, 0xc0]),
      frame("tx-a", "outbound", [0x44]),
    ], { mode: "slip", delimiterHex: "0A", includeDelimiter: true, fixedLength: 8, gapMs: 20 });

    expect(result.frames.map((item) => ({
      direction: item.direction,
      bytes: item.bytes,
      wireBytes: item.wireBytes,
      complete: item.complete,
      sources: item.sourceFrameIds,
    }))).toEqual([
      {
        direction: "inbound",
        bytes: [0x41, 0xc0, 0x42, 0xdb],
        wireBytes: [0xc0, 0x41, 0xdb, 0xdc, 0x42, 0xdb, 0xdd, 0xc0],
        complete: true,
        sources: ["rx-a", "rx-b"],
      },
      {
        direction: "inbound",
        bytes: [0x43],
        wireBytes: [0xc0, 0x43, 0xc0],
        complete: true,
        sources: ["rx-b"],
      },
      {
        direction: "outbound",
        bytes: [0x44],
        wireBytes: [0x44],
        complete: false,
        sources: ["tx-a"],
      },
    ]);
    expect(filterSerialAnalyzedFrames(result.frames, "all", "DB DC")).toEqual([result.frames[0]]);
  });

  it("reports malformed SLIP escapes without losing the decoded packet", () => {
    const result = analyzeSerialCaptureFrames([
      frame("bad", "inbound", [0xc0, 0x41, 0xdb, 0x01, 0xc0]),
    ], { mode: "slip", delimiterHex: "0A", includeDelimiter: true, fixedLength: 8, gapMs: 20 });
    expect(result.frames).toHaveLength(1);
    expect(result.frames[0]).toMatchObject({
      bytes: [0x41, 0x01],
      wireBytes: [0xc0, 0x41, 0xdb, 0x01, 0xc0],
      complete: true,
      decodeError: "invalidEscape",
    });
  });

  it("keeps a dangling SLIP escape in wire evidence without inventing a decoded byte", () => {
    const result = analyzeSerialCaptureFrames([
      frame("tail", "inbound", [0xc0, 0x41, 0xdb]),
    ], { mode: "slip", delimiterHex: "0A", includeDelimiter: true, fixedLength: 8, gapMs: 20 });
    expect(result.frames[0]).toMatchObject({
      bytes: [0x41],
      wireBytes: [0xc0, 0x41, 0xdb],
      complete: false,
      decodeError: "",
    });
  });

  it("does not combine unfinished SLIP payloads across RX and TX", () => {
    const result = analyzeSerialCaptureFrames([
      frame("rx", "inbound", [0x41]),
      frame("tx", "outbound", [0x42, 0xc0]),
    ], { mode: "slip", delimiterHex: "0A", includeDelimiter: true, fixedLength: 8, gapMs: 20 });
    expect(result.frames.map((item) => [item.direction, item.bytes, item.complete])).toEqual([
      ["inbound", [0x41], false],
      ["outbound", [0x42], true],
    ]);
  });

  it("decodes COBS frames across capture chunks and preserves empty payloads and wire bytes", () => {
    const result = analyzeSerialCaptureFrames([
      frame("rx-a", "inbound", [0x03, 0x11]),
      frame("rx-b", "inbound", [0x22, 0x02, 0x33, 0x00, 0x01, 0x00, 0x00]),
      frame("tx", "outbound", [0x03, 0x44, 0x00, 0x02, 0x55]),
    ], { mode: "cobs", delimiterHex: "0A", includeDelimiter: true, fixedLength: 8, gapMs: 20 });

    expect(result.frames.map((item) => ({
      direction: item.direction,
      bytes: item.bytes,
      wireBytes: item.wireBytes,
      complete: item.complete,
      error: item.decodeError,
      sources: item.sourceFrameIds,
    }))).toEqual([
      {
        direction: "inbound",
        bytes: [0x11, 0x22, 0x00, 0x33],
        wireBytes: [0x03, 0x11, 0x22, 0x02, 0x33, 0x00],
        complete: true,
        error: "",
        sources: ["rx-a", "rx-b"],
      },
      {
        direction: "inbound",
        bytes: [],
        wireBytes: [0x01, 0x00],
        complete: true,
        error: "",
        sources: ["rx-b"],
      },
      {
        direction: "outbound",
        bytes: [],
        wireBytes: [0x03, 0x44, 0x00],
        complete: true,
        error: "truncatedCobs",
        sources: ["tx"],
      },
      {
        direction: "outbound",
        bytes: [0x55],
        wireBytes: [0x02, 0x55],
        complete: false,
        error: "",
        sources: ["tx"],
      },
    ]);
    expect(filterSerialAnalyzedFrames(result.frames, "all", "03 11")).toEqual([result.frames[0]]);
  });

  it("does not combine unfinished COBS payloads across RX and TX", () => {
    const result = analyzeSerialCaptureFrames([
      frame("rx", "inbound", [0x02, 0x41]),
      frame("tx", "outbound", [0x02, 0x42, 0x00]),
    ], { mode: "cobs", delimiterHex: "0A", includeDelimiter: true, fixedLength: 8, gapMs: 20 });
    expect(result.frames.map((item) => [item.direction, item.bytes, item.complete])).toEqual([
      ["inbound", [0x41], false],
      ["outbound", [0x42], true],
    ]);
  });

  it("groups and decodes Modbus RTU frames with baud-derived silence", () => {
    const result = analyzeSerialCaptureFrames([
      frame("request-a", "outbound", [0x01, 0x03, 0x00], "2026-07-15T00:00:00.000Z"),
      frame("request-b", "outbound", [0x00, 0x00, 0x0a, 0xc5, 0xcd], "2026-07-15T00:00:00.001Z"),
      frame("bad-crc", "outbound", [0x01, 0x03, 0x00, 0x00, 0x00, 0x0a, 0xc5, 0x00], "2026-07-15T00:00:00.006Z"),
      frame("exception", "inbound", [0x01, 0x83, 0x02, 0xc0, 0xf1], "2026-07-15T00:00:00.007Z"),
    ], {
      mode: "modbus",
      delimiterHex: "0A",
      includeDelimiter: true,
      fixedLength: 8,
      gapMs: 20,
      modbusAutoGap: true,
      modbusGapMs: 2,
    }, 9_600);

    expect(serialModbusSilenceMs(9_600)).toBe(5);
    expect(serialModbusSilenceMs(19_200)).toBe(2);
    expect(result.frames.map((item) => ({
      direction: item.direction,
      bytes: item.bytes,
      complete: item.complete,
      error: item.decodeError,
      protocol: item.protocol,
      sources: item.sourceFrameIds,
    }))).toEqual([
      {
        direction: "outbound",
        bytes: [0x00, 0x00, 0x00, 0x0a],
        complete: true,
        error: "",
        protocol: { kind: "modbusRtu", address: 1, functionCode: 3, exceptionCode: null },
        sources: ["request-a", "request-b"],
      },
      {
        direction: "outbound",
        bytes: [],
        complete: true,
        error: "modbusCrc",
        protocol: null,
        sources: ["bad-crc"],
      },
      {
        direction: "inbound",
        bytes: [0x02],
        complete: false,
        error: "",
        protocol: { kind: "modbusRtu", address: 1, functionCode: 0x83, exceptionCode: 2 },
        sources: ["exception"],
      },
    ]);
  });

  it("maps Modbus RTU short-frame, reserved-address and CRC failures", () => {
    const result = analyzeSerialCaptureFrames([
      frame("short", "inbound", [0x01, 0x03, 0x00], "2026-07-15T00:00:00.000Z"),
      frame("address", "inbound", [0xf8, 0x03, 0x00, 0x00, 0x00, 0x01, 0x90, 0x63], "2026-07-15T00:00:00.010Z"),
      frame("crc", "inbound", [0x01, 0x03, 0x00, 0x00, 0x00, 0x0a, 0xc5, 0x00], "2026-07-15T00:00:00.020Z"),
    ], {
      mode: "modbus",
      delimiterHex: "0A",
      includeDelimiter: true,
      fixedLength: 8,
      gapMs: 20,
      modbusAutoGap: false,
      modbusGapMs: 5,
    });
    expect(result.frames.map((item) => item.decodeError)).toEqual([
      "modbusTooShort",
      "modbusAddress",
      "modbusCrc",
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
