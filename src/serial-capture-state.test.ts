import { describe, expect, it } from "vitest";
import {
  filterSerialCaptureFrames,
  mergeSerialCaptureSnapshot,
  serialCaptureAscii,
  serialCaptureHex,
} from "./serial-capture-state";
import type { SerialCaptureFrame } from "./types";

const frames: SerialCaptureFrame[] = [
  { id: "rx-1", ts: "2026-07-14T00:00:00Z", direction: "inbound", bytes: [0xff, 0x00, 0x80], originalLength: 3, truncated: false },
  { id: "tx-1", ts: "2026-07-14T00:00:01Z", direction: "outbound", bytes: [0x48, 0x69, 0x0d, 0x0a], originalLength: 4, truncated: false },
];

describe("serial capture state", () => {
  it("filters exact binary frames by direction and compact Hex", () => {
    expect(filterSerialCaptureFrames(frames, "inbound", "FF 00 80")).toEqual([frames[0]]);
    expect(filterSerialCaptureFrames(frames, "outbound", "FF 00")).toEqual([]);
  });

  it("searches the escaped ASCII preview case-insensitively", () => {
    expect(filterSerialCaptureFrames(frames, "all", "hi\\r\\n")).toEqual([frames[1]]);
  });

  it("formats non-UTF8 bytes without lossy re-encoding", () => {
    expect(serialCaptureHex(frames[0].bytes)).toBe("FF 00 80");
    expect(serialCaptureAscii(frames[0].bytes)).toBe("...");
  });

  it("marks screen-only Hex truncation without changing frame bytes", () => {
    expect(serialCaptureHex([0, 1, 2, 3], 2)).toBe("00 01 ... (+2 B)");
  });

  it("merges incremental snapshots and resets after ring eviction", () => {
    expect(mergeSerialCaptureSnapshot([frames[0]], {
      frames: [frames[1]],
      reset: false,
      totalFrames: 2,
      capturedBytes: 7,
    })).toEqual(frames);
    expect(mergeSerialCaptureSnapshot(frames, {
      frames: [frames[1]],
      reset: true,
      totalFrames: 1,
      capturedBytes: 4,
    })).toEqual([frames[1]]);
  });

  it("reuses the current frame array when an incremental poll has no changes", () => {
    const unchanged = mergeSerialCaptureSnapshot(frames, {
      frames: [],
      reset: false,
      totalFrames: frames.length,
      capturedBytes: 7,
    });
    expect(unchanged).toBe(frames);
  });
});
