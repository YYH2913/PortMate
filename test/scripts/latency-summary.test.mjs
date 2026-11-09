import { describe, expect, it } from "vitest";
import { latencySummary } from "../tooling/latency-summary.mjs";

describe("tail-latency reporting", () => {
  it("uses nearest rank and exposes a severe outlier even when P99 misses it", () => {
    const values = Array(1000).fill(16);
    values[999] = 1500;
    expect(latencySummary(values)).toEqual({ samples: 1000, p50: 16, p95: 16, p99: 16, max: 1500, over50: 1, over100: 1, over250: 1 });
  });
  it("does not interpolate or hide invalid and undersized samples", () => {
    expect(latencySummary([1, 2, 3, 4, 5]).p99).toBe(5);
    expect(latencySummary(Array.from({ length: 1000 }, (_, i) => i + 1)).p99).toBe(990);
    for (const values of [[], [NaN], [-1], [Infinity]]) expect(() => latencySummary(values)).toThrow();
  });
});
