import { describe, expect, it } from "vitest";
import { MAX_SERIALIZED_TERMINAL_EVENTS, TerminalStateCache } from "./terminal-state-cache";

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
    expect(cache.save("a", { serialized: "screen", cols: 0, rows: Number.NaN, seenEventIds: eventIds })).toBe(true);
    const restored = cache.get("a")!;
    restored.seenEventIds.push("mutated");

    expect(restored.cols).toBe(1);
    expect(restored.rows).toBe(1);
    expect(restored.seenEventIds).toHaveLength(MAX_SERIALIZED_TERMINAL_EVENTS + 1);
    expect(cache.get("a")?.seenEventIds).toHaveLength(MAX_SERIALIZED_TERMINAL_EVENTS);
    expect(cache.get("a")?.seenEventIds[0]).toBe("event-2");
  });

  it("retains an empty serialized screen with its dimensions", () => {
    const cache = new TerminalStateCache();
    expect(cache.save("blank", state(""))).toBe(true);
    expect(cache.get("blank")).toMatchObject({ serialized: "", cols: 80, rows: 24 });
  });
});

function state(serialized: string) {
  return { serialized, cols: 80, rows: 24, seenEventIds: [] };
}
