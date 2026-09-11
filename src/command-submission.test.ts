import { describe, expect, it, vi } from "vitest";
import { extractSubmittedCommands, recordCommandSubmission } from "./command-submission";
import { queuePendingCommandHistory } from "./command-history-state";

describe("command submission boundary", () => {
  it("extracts complete lines from send panel and MCP text", () => {
    expect(extractSubmittedCommands("one\r\ntwo\npartial", "send-panel")).toEqual(["one", "two", "partial"]);
    expect(extractSubmittedCommands("echo ok", "mcp-send-text")).toEqual([]);
    expect(extractSubmittedCommands("echo ok\n", "mcp-send-text")).toEqual(["echo ok"]);
  });
  it("preserves the action source through the recorder and startup queue", () => {
    const record = vi.fn();
    recordCommandSubmission(record, "serial", "echo hello", "paste");
    expect(record).toHaveBeenCalledExactlyOnceWith("serial", "echo hello", "paste");
    const pending = queuePendingCommandHistory([], "echo hello", { limit: 10, retentionDays: 30 }, 1, "serial", "paste");
    expect(pending[0]).toEqual({ command: "echo hello", sessionId: "serial", source: "paste" });
    recordCommandSubmission(record, "serial", "bad\0input", "interactive");
    expect(record).toHaveBeenCalledTimes(1);
  });
});
