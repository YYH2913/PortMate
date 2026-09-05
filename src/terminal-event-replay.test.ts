import { describe, expect, it } from "vitest";
import { TerminalReplayBoundary, terminalHistorySuffix } from "./terminal-event-replay";
import { TerminalStateCache } from "./terminal-state-cache";
import type { SessionEvent } from "./types";

function event(id: string, ts = "2026-09-05T09:00:00.000000Z"): SessionEvent {
  return { id, ts, sessionId: "a", paneId: "a:main", direction: "inbound", stream: "stdout", text: id, bytesRef: null, annotations: {} };
}

describe("terminal replay boundaries", () => {
  it("skips shifted and expanded history prefixes even after their ids were evicted", () => {
    const events = ["old", "a", "b", "live", "new"].map((id) => event(id));
    expect(terminalHistorySuffix(events, ["a", "b"], new Set(["live"]))).toEqual([events[4]]);
    expect(terminalHistorySuffix(events.slice(0, 3), ["a", "b"], new Set())).toEqual([]);
    expect(terminalHistorySuffix(events, [], new Set())).toEqual(events);
  });

  it("rejects old polling and cached packets without depending on a bounded id set", () => {
    const boundary = new TerminalReplayBoundary();
    expect(boundary.accept(event("history"), true)).toBe(true);
    expect(boundary.accept(event("live", "2026-09-05T09:00:01.123456Z"), false)).toBe(true);
    expect(boundary.accept(event("late-history"), true)).toBe(false);
    expect(boundary.accept(event("older-microsecond", "2026-09-05T09:00:01.123455Z"), true)).toBe(false);
    expect(boundary.accept(event("equal", "2026-09-05T17:00:01.123456+08:00"), true)).toBe(true);
    expect(boundary.accept(event("poll-only", "2026-09-05T09:00:02Z"), true)).toBe(true);
  });

  it("does not suppress fresh output after a wall-clock rollback or an outbound input", () => {
    const boundary = new TerminalReplayBoundary();
    expect(boundary.accept({ ...event("input", "2026-09-05T19:00:00Z"), direction: "outbound" }, false)).toBe(true);
    expect(boundary.timestamp).toBeNull();
    expect(boundary.accept(event("first"), true)).toBe(true);
    expect(boundary.accept(event("clock-adjusted", "2026-09-05T05:00:00Z"), false)).toBe(true);
  });

  it("persists the replay boundary across a serialized terminal remount", () => {
    const cache = new TerminalStateCache();
    cache.save("a", { serialized: "prompt", cols: 80, rows: 24, seenEventIds: [], replayTimestamp: "2026-09-05T09:00:01.123456789Z" });
    const boundary = new TerminalReplayBoundary(cache.get("a")?.replayTimestamp);
    expect(boundary.accept(event("delayed-poll"), true)).toBe(false);
    expect(boundary.accept(event("new", "2026-09-05T09:00:02Z"), false)).toBe(true);
  });
});
