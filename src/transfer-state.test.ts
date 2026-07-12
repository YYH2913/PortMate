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
