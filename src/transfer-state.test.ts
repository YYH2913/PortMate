import { describe, expect, it } from "vitest";
import type { TransferTask } from "./types";
import { mergeTransfers } from "./transfer-state";

function transfer(patch: Partial<TransferTask> = {}): TransferTask {
  return {
    id: "transfer-1",
    sessionId: "session-1",
    protocol: "sftp",
    source: "source.bin",
    destination: "destination.bin",
    bytesTotal: 0,
    bytesDone: 0,
    status: "queued",
    message: "queued",
    startedAt: null,
    finishedAt: null,
    averageBytesPerSecond: null,
    ...patch,
  };
}

describe("mergeTransfers", () => {
  it("appends a new transfer", () => {
    expect(mergeTransfers([], transfer())).toHaveLength(1);
  });

  it("does not regress a terminal task to a queued command response", () => {
    const completed = transfer({ status: "completed", bytesTotal: 7, bytesDone: 7, message: "completed" });
    expect(mergeTransfers([completed], transfer())[0]).toBe(completed);
  });

  it("fills metadata from a later snapshot of the same terminal state", () => {
    const completed = transfer({ status: "completed", bytesDone: 7, message: null });
    const finalized = transfer({
      status: "completed",
      bytesTotal: 7,
      bytesDone: 7,
      message: "completed",
      startedAt: "2026-07-31T01:59:58.000Z",
      finishedAt: "2026-07-31T02:00:00.000Z",
      averageBytesPerSecond: 3.5,
    });
    expect(mergeTransfers([completed], finalized)[0]).toEqual(finalized);
  });

  it("does not regress terminal progress or replace one terminal state with another", () => {
    const failed = transfer({
      status: "failed",
      bytesTotal: 100,
      bytesDone: 80,
      message: "permission denied",
      finishedAt: "2026-07-31T02:00:00.000Z",
      averageBytesPerSecond: 40,
    });
    const stale = transfer({
      status: "failed",
      bytesTotal: 20,
      bytesDone: 10,
      message: "failed",
      averageBytesPerSecond: 10,
    });
    expect(mergeTransfers([failed], stale)[0]).toBe(failed);
    expect(mergeTransfers([failed], transfer({ status: "cancelled" }))[0]).toBe(failed);
  });

  it("does not regress running progress or known totals", () => {
    const running = transfer({ status: "running", bytesTotal: 100, bytesDone: 40, startedAt: "2026-07-12T00:00:00Z" });
    expect(mergeTransfers([running], transfer({ status: "running", bytesDone: 20 }))[0]).toBe(running);
    expect(mergeTransfers([running], transfer({ status: "running", bytesDone: 40 }))[0]).toBe(running);
  });

  it("accepts forward status and progress updates", () => {
    const queued = transfer();
    const running = transfer({ status: "running", bytesTotal: 100, bytesDone: 10, startedAt: "2026-07-12T00:00:00Z" });
    expect(mergeTransfers([queued], running)[0]).toBe(running);
    const progressed = transfer({ ...running, bytesDone: 80 });
    expect(mergeTransfers([running], progressed)[0]).toBe(progressed);
  });
});
