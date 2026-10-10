import { describe, expect, it } from "vitest";
import type { SessionSummary } from "../../src/types";
import {
  DEFAULT_SEND_COUNT,
  DEFAULT_SEND_INTERVAL_MS,
  DEFAULT_SEND_LINE_ENDING,
  MAX_SEND_COUNT,
  MAX_SEND_INTERVAL_MS,
  appendSendLineEnding,
  dispatchPacedSends,
  normalizeSendCount,
  normalizeSendInterval,
  parseHexBytes,
  resolveSendTargets,
} from "../../src/send-panel-state";

describe("send panel line endings", () => {
  it("preserves raw text by default", () => {
    expect(DEFAULT_SEND_LINE_ENDING).toBe("none");
    expect(appendSendLineEnding("one\r\ntwo\nthree\r", DEFAULT_SEND_LINE_ENDING)).toBe("one\r\ntwo\nthree\r");
  });

  it.each([
    ["none", []],
    ["lf", [10]],
    ["cr", [13]],
    ["crlf", [13, 10]],
  ] as const)("appends exactly the %s bytes to UTF-8 text", (ending, bytes) => {
    const draft = "中😀\n";
    const prefix = [...new TextEncoder().encode(draft)];
    const payload = appendSendLineEnding(draft, ending);
    expect([...new TextEncoder().encode(payload)]).toEqual([...prefix, ...bytes]);
    expect(appendSendLineEnding(draft, ending)).toBe(payload);
    expect(draft).toBe("中😀\n");
  });

  it.each([
    ["none", ""], ["lf", "\n"], ["cr", "\r"], ["crlf", "\r\n"],
  ] as const)("supports an empty draft with %s selected", (ending, expected) => {
    expect(appendSendLineEnding("", ending)).toBe(expected);
  });

  it("includes the suffix in the UTF-8 payload size", () => {
    const limit = 4 * 1024 * 1024;
    const draft = "a".repeat(limit - 1);
    expect(new TextEncoder().encode(appendSendLineEnding(draft, "lf")).length).toBe(limit);
    expect(new TextEncoder().encode(appendSendLineEnding(draft, "crlf")).length).toBe(limit + 1);
  });
});

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
