import { describe, expect, it } from "vitest";
import { reconcileSessionEventUpdates } from "../../src/session-event-updates";
import type { SessionEvent } from "../../src/types";

function event(id: string, sessionId = "a", bytesRef: string | null = null): SessionEvent {
  return { id, sessionId, bytesRef, text: id, paneId: `${sessionId}:main`, ts: "2026-11-07T00:00:00Z", stream: "stdout", direction: "inbound", annotations: {} };
}
describe("batched session metadata updates", () => {
  it("keeps the last update, original order and untouched object identities", () => {
    const one = event("one"), two = event("two");
    const current = { a: [one, two], b: [event("three", "b")] };
    const updated = event("two", "a", "raw:new");
    const result = reconcileSessionEventUpdates(current, [event("two", "a", "old"), updated, event("unknown"), event("missing", "not-hydrated")]);
    expect(result.changedSessions).toEqual(["a"]);
    expect(result.logs).toEqual({ a: [one, updated], b: current.b });
    expect(result.logs.a[0]).toBe(one);
    expect(result.logs.b).toBe(current.b);
    expect(current.a[1]).toBe(two);
    expect(reconcileSessionEventUpdates(current, [two, event("unknown")]).logs).toBe(current);
  });
  it("scans an existing log only once for a large metadata burst", () => {
    let reads = 0;
    const events = Array.from({ length: 5_000 }, (_, i) => {
      const value = event(String(i));
      Object.defineProperty(value, "id", { get: () => { reads++; return String(i); } });
      return value;
    });
    const updates = events.map((_, i) => event(String(i), "a", `raw:${i}`));
    reads = 0;
    const result = reconcileSessionEventUpdates({ a: events }, updates);
    expect(reads).toBe(5_000);
    expect(result.logs.a).toEqual(updates);
  });
});
