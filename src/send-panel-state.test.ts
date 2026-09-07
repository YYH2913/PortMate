import { describe, expect, it } from "vitest";
import type { SessionSummary } from "./types";
import {
  DEFAULT_SEND_COUNT,
  DEFAULT_SEND_INTERVAL_MS,
  MAX_SEND_COUNT,
  MAX_SEND_INTERVAL_MS,
  dispatchPacedSends,
  normalizeSendCount,
  normalizeSendInterval,
  parseHexBytes,
  resolveSendTargets,
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

  it("waits a full interval after each acknowledged batch", async () => {
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
    expect(starts).toEqual([0, 130, 260]);
    expect(waits).toEqual([100, 100]);
  });

  it("never catches up in a burst after a slow or queued write", async () => {
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
    expect(starts).toEqual([0, 250]);
    expect(waits).toEqual([100]);
  });

  it("propagates a failed batch and stops subsequent sends", async () => {
    const calls: number[] = [];
    await expect(dispatchPacedSends(3, 0, async (index) => {
      calls.push(index);
      if (index === 1) throw new Error("write failed");
    })).rejects.toThrow("write failed");
    expect(calls).toEqual([0, 1]);
  });

  it("cancels an interval before the next repeated send", async () => {
    const controller = new AbortController();
    const calls: number[] = [];
    let releaseWait!: () => void;
    const waiting = new Promise<void>((resolve) => { releaseWait = resolve; });
    const dispatch = dispatchPacedSends(
      3,
      100,
      async (index) => { calls.push(index); },
      async (_milliseconds, signal) => {
        await waiting;
        signal?.throwIfAborted();
      },
      () => 0,
      { signal: controller.signal },
    );

    await expect.poll(() => calls).toEqual([0]);
    controller.abort();
    releaseWait();

    await expect(dispatch).rejects.toMatchObject({ name: "AbortError" });
    expect(calls).toEqual([0]);
  });

  it("rejects invalid Hex without stripping input or shifting byte boundaries", () => {
    expect(parseHexBytes("01 FF, 0x02 0Xaa 0304")).toEqual([1, 255, 2, 170, 3, 4]);
    expect(parseHexBytes("  ")).toEqual([]);
    for (const value of ["01 GG 02", "1 02", "123", "0x", "01!02", "hello", "0x010x02"]) {
      expect(() => parseHexBytes(value)).toThrow("Hex 格式无效");
    }
  });

  it("deduplicates repeated panes and excludes disconnected targets", () => {
    const connected: SessionSummary = { profile: { id: "s" } as SessionSummary["profile"],
      runtime: { status: "connected" } as SessionSummary["runtime"], logLines: 0 };
    const disconnected: SessionSummary = { profile: { id: "d" } as SessionSummary["profile"],
      runtime: { status: "disconnected" } as SessionSummary["runtime"], logLines: 0 };
    const sessions = [connected, disconnected];
    expect(resolveSendTargets("panes", "s", sessions, [sessions[0], sessions[0], sessions[1]])).toEqual(["s"]);
    expect(resolveSendTargets("active", "d", sessions, [])).toEqual([]);
    expect(resolveSendTargets("connected", "", sessions, [])).toEqual(["s"]);
  });
});
