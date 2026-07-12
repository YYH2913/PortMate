import { describe, expect, it } from "vitest";
import type { TransferTask } from "./types";
import { transferDiagnosticText, transferDisplayMessage, transferStatusLabel } from "./transfer-presentation";

function transfer(patch: Partial<TransferTask> = {}): TransferTask {
  return {
    id: "transfer-1",
    sessionId: "session-1",
    protocol: "scp",
    source: "remote:/srv/source.bin",
    destination: "remote:/srv/destination.bin",
    bytesTotal: 1024,
    bytesDone: 256,
    status: "failed",
    message: "remote command exited with status 1",
    startedAt: "2026-07-12T01:00:00Z",
    finishedAt: "2026-07-12T01:00:03Z",
    averageBytesPerSecond: 85.3,
    ...patch,
  };
}

describe("transfer presentation", () => {
  it("localizes every transfer status", () => {
    expect(["queued", "running", "completed", "failed", "cancelled"].map((status) =>
      transferStatusLabel(status as TransferTask["status"]),
    )).toEqual(["排队中", "传输中", "已完成", "失败", "已取消"]);
  });

  it("hides generic lifecycle messages", () => {
    expect(transferDisplayMessage(transfer({ status: "running", message: "running" }))).toBeNull();
    expect(transferDisplayMessage(transfer({ status: "completed", message: "completed" }))).toBeNull();
  });

  it("provides a fallback for failures without details", () => {
    expect(transferDisplayMessage(transfer({ message: null }))).toBe("传输失败，远端未返回详细原因");
  });

  it("builds a copyable diagnostic with partial progress and error detail", () => {
    const diagnostic = transferDiagnosticText(transfer());
    expect(diagnostic).toContain("Status: 失败 (failed)");
    expect(diagnostic).toContain("Progress: 256 / 1024 bytes");
    expect(diagnostic).toContain("Source: remote:/srv/source.bin");
    expect(diagnostic).toContain("Message: remote command exited with status 1");
  });
});
