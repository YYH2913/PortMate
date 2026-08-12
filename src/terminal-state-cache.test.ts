import { describe, expect, it } from "vitest";
import {
  MAX_SERIALIZED_TERMINAL_EVENTS,
  rememberTerminalEventId,
  settleTerminalEventId,
  TerminalStateCache,
} from "./terminal-state-cache";

describe("terminal state cache", () => {
  it("evicts the least recently used terminal", () => {
    const cache = new TerminalStateCache(2, 100);
    expect(cache.save("a", state("A"))).toBe(true);
    expect(cache.save("b", state("B"))).toBe(true);
    expect(cache.get("a")?.serialized).toBe("A");
    expect(cache.save("c", state("C"))).toBe(true);

    expect(cache.get("b")).toBeUndefined();
    expect(cache.get("a")?.serialized).toBe("A");
    expect(cache.get("c")?.serialized).toBe("C");
  });

  it("rejects oversized UTF-8 state without replacing a valid snapshot", () => {
    const cache = new TerminalStateCache(2, 4);
    expect(cache.save("a", state("ok"))).toBe(true);
    expect(cache.save("a", state("terminal"))).toBe(false);
    expect(cache.save("emoji", state("😀"))).toBe(true);
    expect(cache.save("overflow", state("😀x"))).toBe(false);

    expect(cache.get("a")?.serialized).toBe("ok");
    expect(cache.get("emoji")?.serialized).toBe("😀");
  });

  it("normalizes dimensions and bounds copied event ids", () => {
    const cache = new TerminalStateCache();
    const eventIds = Array.from({ length: MAX_SERIALIZED_TERMINAL_EVENTS + 2 }, (_, index) => `event-${index}`);
    expect(cache.save("a", {
      serialized: "screen",
      cols: 0,
      rows: Number.NaN,
      seenEventIds: eventIds,
      mouseEncoding: "sgr",
      timestamps: [
        { line: 7, ts: "2026-08-09T01:02:03Z" },
        { line: 7, ts: "2026-08-09T01:02:04Z" },
        { line: -1, ts: "invalid" },
      ],
      alternateTimestamp: "2026-08-09T01:02:06.123456789Z",
    })).toBe(true);
    const restored = cache.get("a")!;
    restored.seenEventIds.push("mutated");
    restored.timestamps?.push({ line: 8, ts: "2026-08-09T01:02:05.000000Z" });
    restored.alternateTimestamps?.push({ line: 8, ts: "2026-08-09T01:02:07.000000Z" });

    expect(restored.cols).toBe(1);
    expect(restored.rows).toBe(1);
    expect(restored.mouseEncoding).toBe("sgr");
    expect(restored.seenEventIds).toHaveLength(MAX_SERIALIZED_TERMINAL_EVENTS + 1);
    expect(cache.get("a")?.seenEventIds).toHaveLength(MAX_SERIALIZED_TERMINAL_EVENTS);
    expect(cache.get("a")?.seenEventIds[0]).toBe("event-2");
    expect(cache.get("a")?.timestamps).toEqual([
      { line: 7, ts: "2026-08-09T01:02:03.000000Z" },
    ]);
    expect(cache.get("a")?.alternateTimestamp).toBe("2026-08-09T01:02:06.123456Z");
    expect(cache.get("a")?.alternateTimestamps).toEqual([
      { line: 0, ts: "2026-08-09T01:02:06.123456Z" },
    ]);
  });

  it("normalizes, resizes, and deep-clones alternate-screen row timestamps", () => {
    const cache = new TerminalStateCache();
    expect(cache.save("alternate", {
      serialized: "screen",
      cols: 80,
      rows: 4,
      seenEventIds: [],
      alternateTimestamps: [
        { line: 0, ts: "2026-08-09T01:02:01.111111789Z" },
        { line: 1, ts: "2026-08-09T01:02:02.222222789Z" },
        { line: 3, ts: "invalid" },
        { line: 8, ts: "2026-08-09T01:02:08Z" },
      ],
      alternateTimestamp: "2026-08-09T01:02:03.333333789Z",
    })).toBe(true);

    const restored = cache.get("alternate")!;
    expect(restored.alternateTimestamps).toEqual([
      { line: 0, ts: "2026-08-09T01:02:01.111111Z" },
      { line: 1, ts: "2026-08-09T01:02:02.222222Z" },
      { line: 2, ts: "2026-08-09T01:02:03.333333Z" },
      { line: 3, ts: "2026-08-09T01:02:03.333333Z" },
    ]);
    restored.alternateTimestamps![0].ts = "2026-08-09T01:02:09.999999Z";
    expect(cache.get("alternate")?.alternateTimestamps?.[0].ts)
      .toBe("2026-08-09T01:02:01.111111Z");
  });

  it("retains an empty serialized screen with its dimensions", () => {
    const cache = new TerminalStateCache();
    expect(cache.save("blank", state(""))).toBe(true);
    expect(cache.get("blank")).toMatchObject({ serialized: "", cols: 80, rows: 24, mouseEncoding: "default" });
  });

  it("evicts event ids in order without replaying the recent polling window", () => {
    const seen = new Set<string>();
    const pending = new Set<string>();
    for (let index = 0; index < MAX_SERIALIZED_TERMINAL_EVENTS + 100; index += 1) {
      const eventId = `event-${index}`;
      expect(rememberTerminalEventId(seen, pending, eventId)).toBe(true);
      settleTerminalEventId(seen, pending, eventId);
    }

    expect(seen.size).toBe(MAX_SERIALIZED_TERMINAL_EVENTS);
    expect(seen.has("event-0")).toBe(false);
    expect(seen.has("event-99")).toBe(false);
    expect(seen.has("event-100")).toBe(true);
    for (let index = 3500; index < 4100; index += 1) {
      expect(rememberTerminalEventId(seen, pending, `event-${index}`)).toBe(false);
    }
    expect(pending.size).toBe(0);
    expect(seen.size).toBe(MAX_SERIALIZED_TERMINAL_EVENTS);
  });

  it("does not evict terminal events while their XTerm writes are pending", () => {
    const seen = new Set<string>();
    const pending = new Set<string>();
    for (const eventId of ["a", "b", "c"]) {
      expect(rememberTerminalEventId(seen, pending, eventId, 2)).toBe(true);
    }
    expect([...seen]).toEqual(["a", "b", "c"]);

    settleTerminalEventId(seen, pending, "a", 2);
    expect([...seen]).toEqual(["b", "c"]);
    settleTerminalEventId(seen, pending, "b", 2);
    settleTerminalEventId(seen, pending, "c", 2);
    expect(pending.size).toBe(0);
    expect(rememberTerminalEventId(seen, pending, "c", 2)).toBe(false);
  });
});

function state(serialized: string) {
  return { serialized, cols: 80, rows: 24, seenEventIds: [] };
}
