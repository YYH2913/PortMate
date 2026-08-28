import { describe, expect, it } from "vitest";
import {
  DEFAULT_SEND_COUNT,
  DEFAULT_SEND_INTERVAL_MS,
  MAX_SEND_COUNT,
  MAX_SEND_INTERVAL_MS,
  dispatchPacedSends,
  normalizeSendCount,
  normalizeSendInterval,
} from "./send-panel-state";

describe("send panel pacing", () => {
  it("normalizes invalid and bounded sender settings", () => {
    expect(normalizeSendCount(Number.NaN)).toBe(DEFAULT_SEND_COUNT);
    expect(normalizeSendCount(2.9)).toBe(2);
    expect(normalizeSendCount(999_999)).toBe(MAX_SEND_COUNT);
    expect(normalizeSendInterval(Number.POSITIVE_INFINITY)).toBe(DEFAULT_SEND_INTERVAL_MS);
    expect(normalizeSendInterval(-10)).toBe(0);
    expect(normalizeSendInterval(12.9)).toBe(12);
    expect(normalizeSendInterval(999_999_999)).toBe(MAX_SEND_INTERVAL_MS);
  });

  it("waits from one batch start to the next", async () => {
    let clock = 0;
    const waits: number[] = [];
    const starts: number[] = [];
    await dispatchPacedSends(
      3,
      100,
      async () => {
        starts.push(clock);
        clock += 30;
      },
      async (milliseconds) => {
        waits.push(milliseconds);
        clock += milliseconds;
      },
      () => clock,
    );
    expect(starts).toEqual([0, 100, 200]);
    expect(waits).toEqual([70, 70]);
  });

  it("does not add an extra wait after a slow write", async () => {
    let clock = 0;
    const waits: number[] = [];
    const starts: number[] = [];
    await dispatchPacedSends(
      2,
      100,
      async () => {
        starts.push(clock);
        clock += 150;
      },
      async (milliseconds) => {
        waits.push(milliseconds);
        clock += milliseconds;
      },
      () => clock,
    );
    expect(starts).toEqual([0, 150]);
    expect(waits).toEqual([]);
  });

  it("propagates a failed batch and stops subsequent sends", async () => {
    const calls: number[] = [];
    await expect(dispatchPacedSends(3, 0, async (index) => {
      calls.push(index);
      if (index === 1) throw new Error("write failed");
    })).rejects.toThrow("write failed");
    expect(calls).toEqual([0, 1]);
  });
});
