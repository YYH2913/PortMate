import { afterEach, describe, expect, it } from "vitest";
import {
  appendTerminalByteCacheEvent,
  appendTerminalByteCacheEvents,
  appendTerminalByteEvent,
  emptyTerminalByteBuffer,
  moveTerminalByteSelection,
  readTerminalDisplayMode,
  resetTerminalByteCacheForTests,
  subscribeTerminalByteCache,
  terminalByteAscii,
  terminalByteCacheSnapshot,
  terminalByteFollowForScroll,
  terminalByteRows,
  terminalByteSelectionPosition,
  writeTerminalDisplayMode,
} from "./terminal-byte-state";
import type { TerminalBytesEvent } from "./types";

function byteEvent(
  id: string,
  bytes: number[],
  overrides: Partial<TerminalBytesEvent> = {},
): TerminalBytesEvent {
  return {
    id,
    sessionId: "session-a",
    ts: "2026-08-05T00:00:00.000Z",
    direction: "inbound",
    stream: "stdout",
    bytes,
    originalLength: bytes.length,
    truncated: false,
    eventId: null,
    ...overrides,
  };
}

afterEach(() => resetTerminalByteCacheForTests());

describe("terminal byte buffer", () => {
  it("preserves non-UTF-8 bytes without text re-encoding", () => {
    const buffer = appendTerminalByteEvent(emptyTerminalByteBuffer(), byteEvent("binary", [0x00, 0x80, 0xff, 0x41]));
    expect(buffer.frames[0].bytes).toEqual([0x00, 0x80, 0xff, 0x41]);
    expect(buffer.capturedBytes).toBe(4);
  });

  it("renders CR, LF, TAB and non-printable bytes explicitly", () => {
    expect(terminalByteAscii([0x41, 0x0d, 0x0a, 0x09, 0x80])).toBe("A\\r\\n\\t.");
  });

  it("uses stable 16-byte row offsets across frames", () => {
    let buffer = appendTerminalByteEvent(emptyTerminalByteBuffer(), byteEvent("first", Array.from({ length: 20 }, (_, index) => index)));
    buffer = appendTerminalByteEvent(buffer, byteEvent("second", [0xaa, 0xbb], { direction: "outbound" }));
    const rows = terminalByteRows(buffer.frames);
    expect(rows.map((row) => row.offset)).toEqual([0, 16, 20]);
    expect(rows.map((row) => row.bytes.length)).toEqual([16, 4, 2]);
  });

  it("evicts the oldest frames within byte and frame limits", () => {
    const limits = { maxBytes: 5, maxFrames: 2 };
    let buffer = appendTerminalByteEvent(emptyTerminalByteBuffer(), byteEvent("first", [1, 2, 3]), limits);
    buffer = appendTerminalByteEvent(buffer, byteEvent("second", [4, 5, 6]), limits);
    expect(buffer.frames.map((frame) => frame.id)).toEqual(["second"]);
    expect(buffer.capturedBytes).toBe(3);
    expect(buffer.droppedBytes).toBe(3);
    expect(buffer.droppedFrames).toBe(1);
    expect(buffer.nextOffset).toBe(6);
  });

  it("deduplicates a live event shared by multiple terminal views", () => {
    appendTerminalByteCacheEvent(byteEvent("shared", [1, 2, 3]));
    appendTerminalByteCacheEvent(byteEvent("shared", [1, 2, 3]));
    expect(terminalByteCacheSnapshot("session-a").frames).toHaveLength(1);
  });

  it("notifies a session subscriber once for a burst of frames", () => {
    let notifications = 0;
    const stop = subscribeTerminalByteCache("session-a", () => { notifications += 1; });
    appendTerminalByteCacheEvents([
      byteEvent("burst-1", [1]),
      byteEvent("burst-2", [2]),
      byteEvent("burst-3", [3]),
    ]);
    stop();
    expect(notifications).toBe(1);
    expect(terminalByteCacheSnapshot("session-a").capturedBytes).toBe(3);
  });
});

describe("terminal byte interaction", () => {
  it("moves one selection across frame boundaries for Hex and ASCII correspondence", () => {
    let buffer = appendTerminalByteEvent(emptyTerminalByteBuffer(), byteEvent("first", [1, 2]));
    buffer = appendTerminalByteEvent(buffer, byteEvent("second", [3, 4]));
    const start = { frameId: "first", byteIndex: 1 };
    const next = moveTerminalByteSelection(buffer.frames, start, 1);
    expect(next).toEqual({ frameId: "second", byteIndex: 0 });
    expect(terminalByteSelectionPosition(buffer.frames, next)).toBe(2);
  });

  it("enables follow only while the viewport remains near the latest row", () => {
    expect(terminalByteFollowForScroll(752, 200, 1000)).toBe(true);
    expect(terminalByteFollowForScroll(500, 200, 1000)).toBe(false);
  });

  it("keeps each terminal view display mode independent", () => {
    const values = new Map<string, string>();
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => { values.set(key, value); },
    };
    writeTerminalDisplayMode(storage, "view-a", "split");
    writeTerminalDisplayMode(storage, "view-b", "hex");
    expect(readTerminalDisplayMode(storage, "view-a")).toBe("split");
    expect(readTerminalDisplayMode(storage, "view-b")).toBe("hex");
    expect(readTerminalDisplayMode(storage, "view-c")).toBe("text");
  });
});
