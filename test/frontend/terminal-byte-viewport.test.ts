import { describe, expect, it } from "vitest";
import { indexTerminalByteRows, terminalByteRowRange, terminalByteRows } from "../../src/terminal-byte-state";
import type { TerminalByteFrame } from "../../src/terminal-byte-state";

function frame(id: string, bytes: number[], offset = 0): TerminalByteFrame {
  return { id, sessionId: "s", ts: "2026-11-08T00:00:00Z", direction: "inbound", stream: "stdout", bytes, originalLength: bytes.length, startOffset: offset, truncated: false };
}
describe("virtual byte viewport", () => {
  it("matches full row semantics across frame boundaries, tail fragments and truncation", () => {
    const frames = [frame("first", Array.from({ length: 19 }, (_, i) => i)), frame("empty", []), { ...frame("last", [0x80, 0xff, 0, 10], 19), originalLength: 9, truncated: true }];
    for (const width of [1, 3, 8, 16, 32]) {
      const index = indexTerminalByteRows(frames, width);
      const full = frames.flatMap(frame => Array.from({ length: Math.ceil(frame.bytes.length / width) }, (_, row) => {
        const frameByteOffset = row * width;
        return { id: `${frame.id}:${frameByteOffset}`, frameId: frame.id, ts: frame.ts, direction: frame.direction, stream: frame.stream,
          offset: frame.startOffset + frameByteOffset, frameByteOffset, bytes: Array.from(frame.bytes.slice(frameByteOffset, frameByteOffset + width)),
          omittedBytes: frameByteOffset + width >= frame.bytes.length ? Math.max(0, frame.originalLength - frame.bytes.length) : 0 };
      }));
      expect(terminalByteRows(frames, width)).toEqual(full);
      expect(index.rowCount).toBe(full.length);
      for (let start = 0; start <= full.length; start++) {
        expect(terminalByteRowRange(index, start, 3)).toEqual(full.slice(start, start + 3));
      }
      expect(full.at(-1)?.omittedBytes).toBe(5);
    }
  });
  it("reads only visible bytes in a full 1 MiB capture, never expands the whole cache", () => {
    let reads = 0;
    const frames = Array.from({ length: 64 }, (_, n) => frame(String(n), new Proxy(Array(16_384).fill(n), {
      get(target, key, receiver) { if (/^\d+$/.test(String(key))) reads++; return Reflect.get(target, key, receiver); },
    }), n * 16_384));
    const index = indexTerminalByteRows(frames, 8);
    expect(index.rowCount).toBe(131_072);
    expect(reads).toBe(0);
    const visible = terminalByteRowRange(index, index.rowCount - 40, 40);
    expect(reads).toBe(320);
    expect(visible).toHaveLength(40);
    expect(visible[0].offset).toBe(1_048_576 - 320);
  });
});
