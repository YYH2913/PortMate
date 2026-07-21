import { describe, expect, it } from "vitest";
import { KeyedRequestGate } from "./keyed-request-gate";

describe("keyed request gate", () => {
  it("prevents overlapping requests for the same key", () => {
    const gate = new KeyedRequestGate<string>();
    const first = gate.begin("session-a");

    expect(first).not.toBeNull();
    expect(gate.begin("session-a")).toBeNull();
    expect(gate.begin("session-b")).not.toBeNull();
    expect(gate.finish("session-a", first!)).toBe(true);
    expect(gate.begin("session-a")).not.toBeNull();
  });

  it("does not let an invalidated request finish a replacement", () => {
    const gate = new KeyedRequestGate<string>();
    const stale = gate.begin("session-a")!;
    gate.invalidate("session-a");
    const replacement = gate.begin("session-a")!;

    expect(gate.isCurrent("session-a", stale)).toBe(false);
    expect(gate.finish("session-a", stale)).toBe(false);
    expect(gate.isCurrent("session-a", replacement)).toBe(true);
    expect(gate.finish("session-a", replacement)).toBe(true);
  });

  it("replaces an active request and accepts only the newest response", () => {
    const gate = new KeyedRequestGate<string>();
    const stale = gate.replace("session-a");
    const replacement = gate.replace("session-a");

    expect(gate.isCurrent("session-a", stale)).toBe(false);
    expect(gate.finish("session-a", stale)).toBe(false);
    expect(gate.isCurrent("session-a", replacement)).toBe(true);
    expect(gate.finish("session-a", replacement)).toBe(true);
  });

  it("invalidates every active request during a full snapshot replacement", () => {
    const gate = new KeyedRequestGate<string>();
    const first = gate.begin("session-a")!;
    const second = gate.begin("session-b")!;
    gate.invalidateAll();

    expect(gate.isCurrent("session-a", first)).toBe(false);
    expect(gate.isCurrent("session-b", second)).toBe(false);
  });
});
