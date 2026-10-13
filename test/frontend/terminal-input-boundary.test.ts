import { describe, expect, it, vi } from "vitest";
import { commitTerminalInputAfterWrite, pasteTerminalClipboard, TerminalInputBoundary } from "../../src/terminal-input-boundary";
import type { TerminalInputSnapshot } from "../../src/terminal-input-boundary";
import { TerminalInputController } from "../../src/terminal-input-controller";
import type { SessionSummary } from "../../src/types";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}

function session(status: SessionSummary["runtime"]["status"], connectedSince: string | null = null): SessionSummary {
  return { profile: { id: "serial", kind: "serial" }, runtime: { status, connectedSince } } as SessionSummary;
}

describe("terminal asynchronous input boundaries", () => {
  it("accepts new middle paste after first connect without remounting its boundary", async () => {
    const boundary = new TerminalInputBoundary(session("disconnected"));
    const readText = vi.fn().mockResolvedValue("echo one");
    const paste = vi.fn();
    expect(await pasteTerminalClipboard(boundary, readText, paste, () => true)).toBe(false);
    expect(readText).not.toHaveBeenCalled();
    boundary.update(session("connected", "connection-a"));
    expect(await pasteTerminalClipboard(boundary, readText, paste, () => true)).toBe(true);
    expect(paste).toHaveBeenCalledExactlyOnceWith("echo one", boundary.capture());
  });

  it("drops a pending clipboard read across reconnect and accepts a fresh read", async () => {
    const boundary = new TerminalInputBoundary(session("connected", "connection-a"));
    const read = deferred<string>();
    const paste = vi.fn();
    const pending = pasteTerminalClipboard(boundary, () => read.promise, paste, () => true);
    boundary.update(session("disconnected"));
    boundary.update(session("connected", "connection-b"));
    read.resolve("old clipboard");
    expect(await pending).toBe(false);
    expect(paste).not.toHaveBeenCalled();
    expect(await pasteTerminalClipboard(boundary, async () => "new clipboard", paste, () => true)).toBe(true);
    expect(paste).toHaveBeenCalledExactlyOnceWith("new clipboard", boundary.capture());
  });

  it("fences same-timestamp reconnects, changed session ids, hidden views and disposal", async () => {
    const boundary = new TerminalInputBoundary(session("connected", "same-time"));
    const original = boundary.capture()!;
    boundary.update(session("disconnected"));
    boundary.update(session("connected", "same-time"));
    expect(boundary.isCurrent(original)).toBe(false);
    expect(() => boundary.require("serial", { expectedInputEpoch: original.epoch })).toThrow();
    expect(() => boundary.require("other-session")).toThrow();
    expect(() => boundary.require("serial", { expectedConnectedSince: "other-time" })).toThrow();
    let visible = true;
    const read = deferred<string>();
    const paste = vi.fn();
    const pending = pasteTerminalClipboard(boundary, () => read.promise, paste, () => visible);
    visible = false;
    read.resolve("hidden clipboard");
    expect(await pending).toBe(false);
    boundary.update();
    expect(boundary.capture()).toBeNull();
  });

  it("does not publish rejected clipboard or transport writes as command history", async () => {
    const boundary = new TerminalInputBoundary(session("connected", "connection-a"));
    const paste = vi.fn();
    await expect(pasteTerminalClipboard(boundary, async () => { throw new Error("denied"); }, paste, () => true)).rejects.toThrow("denied");
    expect(paste).not.toHaveBeenCalled();
    const tracker = new TerminalInputController();
    const commands = tracker.track("echo failed\r");
    const record = vi.fn();
    expect(await commitTerminalInputAfterWrite(boundary, boundary.capture()!, async () => { throw new Error("write failed"); },
      () => commands.forEach(command => record(command)), () => tracker.invalidate())).toBe(false);
    expect(record).not.toHaveBeenCalled();
    expect(tracker.snapshot().synchronized).toBe(false);
  });

  it("starts writes synchronously and commits only a current acknowledgement", async () => {
    const boundary = new TerminalInputBoundary(session("connected", "connection-a"));
    const ack = deferred<void>();
    const write = vi.fn(() => ack.promise);
    const commit = vi.fn(), invalidate = vi.fn();
    const pending = commitTerminalInputAfterWrite(boundary, boundary.capture()!, write, commit, invalidate);
    expect(write).toHaveBeenCalledTimes(1);
    expect(commit).not.toHaveBeenCalled();
    boundary.update(session("connected", "connection-b"));
    ack.resolve();
    expect(await pending).toBe(false);
    expect(commit).not.toHaveBeenCalled();
    expect(invalidate).not.toHaveBeenCalled();
    expect(await commitTerminalInputAfterWrite(boundary, boundary.capture()!, async () => {}, commit, invalidate)).toBe(true);
    expect(commit).toHaveBeenCalledTimes(1);
  });

  it("captures before invoking the clipboard API even if readText changes connection synchronously", async () => {
    const boundary = new TerminalInputBoundary(session("connected", "connection-a"));
    let received: TerminalInputSnapshot | null = null;
    expect(await pasteTerminalClipboard(boundary, () => {
      boundary.update(session("connected", "connection-b"));
      return Promise.resolve("old text");
    }, (_text, snapshot) => { received = snapshot; }, () => true)).toBe(false);
    expect(received).toBeNull();
  });
});
