import { describe, expect, it } from "vitest";
import type { IMarker } from "@xterm/xterm";
import { TerminalTimestampIndex } from "./terminal-timestamp-index";

let nextId = 0;
function marker(line: number, read = () => {}) {
  const listeners = new Set<() => void>();
  let disposed = false;
  return {
    id: ++nextId,
    get line() { read(); return line; },
    get isDisposed() { read(); return disposed; },
    move(value: number) { line = value; },
    onDispose(listener: () => void) { listeners.add(listener); return { dispose: () => { listeners.delete(listener); } }; },
    dispose() { if (disposed) return; disposed = true; line = -1; for (const listener of listeners) listener(); listeners.clear(); },
  } satisfies IMarker & { move: (line: number) => void };
}

describe("terminal timestamp index", () => {
  it("inserts redraw/restored intervals in order and keeps the first stamp on a line", () => {
    const index = new TerminalTimestampIndex(100);
    index.add(marker(9), "nine");
    index.add(marker(2), "two");
    index.add(marker(5), "five");
    const duplicate = marker(5);
    expect(index.add(duplicate, "replacement")).toBe(false);
    expect(duplicate.isDisposed).toBe(true);
    expect(index.timestampAt(1)).toBeNull();
    expect(index.timestampAt(8)).toBe("five");
    expect(index.visible(4, 5)).toEqual([{ line: 4, row: 0, ts: "two" }, { line: 5, row: 1, ts: "five" }]);
    expect(index.snapshot()).toEqual([{ line: 2, ts: "two" }, { line: 5, ts: "five" }, { line: 9, ts: "nine" }]);
  });

  it("carries the last evicted boundary forward and follows live marker movement", () => {
    const index = new TerminalTimestampIndex(100);
    const markers = [marker(0), marker(2), marker(6)];
    markers.forEach((entry, i) => index.add(entry, `${i}`));
    markers.forEach(entry => { entry.move(entry.line - 3); if (entry.line < 0) entry.dispose(); });
    expect(index.size).toBe(1);
    expect(index.visible(0, 5)).toEqual([{ line: 0, row: 0, ts: "1" }, { line: 3, row: 3, ts: "2" }]);
    markers[2].move(10); // Insert/reflow before the surviving line.
    expect(index.has(3)).toBe(false);
    expect(index.has(10)).toBe(true);
    expect(index.timestampAt(10)).toBe("2");
  });

  it("cleans interior deletions without discarding unrelated intervals", () => {
    const index = new TerminalTimestampIndex(100);
    const markers = [marker(0), marker(3), marker(6)];
    markers.forEach((entry, i) => index.add(entry, `${i}`));
    markers[1].dispose();
    markers[2].move(5);
    expect(index.snapshot()).toEqual([{ line: 0, ts: "0" }, { line: 5, ts: "2" }]);
    index.removeRange(4, 7);
    expect(markers[2].isDisposed).toBe(true);
    index.add(marker(4), "redraw");
    expect(index.snapshot()).toEqual([{ line: 0, ts: "0" }, { line: 4, ts: "redraw" }]);
  });

  it("bounds intervals and can dispose or reset without stale notifications", () => {
    const index = new TerminalTimestampIndex(2);
    const first = marker(0);
    index.add(first, "first");
    index.add(marker(2), "second");
    index.add(marker(4), "third");
    expect(first.isDisposed).toBe(true);
    expect(index.size).toBe(2);
    expect(index.timestampAt(1)).toBe("first");
    index.dispose();
    expect(index.size).toBe(0);
    expect(index.timestampAt(0)).toBeNull();
    index.add(marker(0), "new");
    expect(index.size).toBe(1);
  });

  it("restores 100,000 distinct intervals linearly and queries only the visible range", () => {
    let reads = 0;
    const index = new TerminalTimestampIndex(100_000);
    for (let line = 0; line < 100_000; line += 1) index.add(marker(line, () => { reads += 1; }), `${line}`);
    expect(reads).toBeLessThan(600_000);
    reads = 0;
    for (let query = 0; query < 1_000; query += 1) {
      expect(index.has(99_999)).toBe(true);
      expect(index.timestampAt(99_999)).toBe("99999");
      expect(index.visible(99_970, 30)).toHaveLength(30);
    }
    expect(reads).toBeLessThan(100_000); // No frame or keypress walks 100k markers.
  });

  it("handles sustained prefix trimming without rescanning retained history", () => {
    let reads = 0;
    const index = new TerminalTimestampIndex(20_000);
    const markers = Array.from({ length: 20_000 }, (_, line) => marker(line, () => { reads += 1; }));
    markers.forEach((entry, i) => index.add(entry, `${i}`));
    reads = 0;
    for (let i = 0; i < 5_000; i += 1) {
      markers[i].move(-1);
      markers[i].dispose();
      expect(index.size).toBe(19_999 - i);
    }
    expect(reads).toBeLessThan(25_000);
    expect(index.anchor).toBe("4999");
  });

  it("cleans repeated prompt erases near the tail without walking old output", () => {
    let reads = 0;
    const index = new TerminalTimestampIndex(100_010);
    for (let line = 0; line < 100_000; line += 1) index.add(marker(line, () => { reads += 1; }), `${line}`);
    let prompt = marker(100_000, () => { reads += 1; });
    index.add(prompt, "prompt");
    index.add(marker(100_001, () => { reads += 1; }), "following");
    reads = 0;
    for (let redraw = 0; redraw < 1_000; redraw += 1) {
      prompt.dispose();
      expect(index.has(100_000)).toBe(false);
      prompt = marker(100_000, () => { reads += 1; });
      index.add(prompt, "redraw");
    }
    expect(reads).toBeLessThan(60_000);
  });
});
