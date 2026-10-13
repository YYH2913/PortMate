import { describe, expect, it, vi } from "vitest";
import { TerminalSessionInput } from "../../src/terminal-session-input";
import type { SessionSummary } from "../../src/types";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}

function session(status: SessionSummary["runtime"]["status"], connectedSince: string | null = null, kind: SessionSummary["profile"]["kind"] = "serial"): SessionSummary {
  return { profile: { id: "serial", kind }, runtime: { status, connectedSince } } as SessionSummary;
}

type Call = { command: string; args: Record<string, unknown> };

describe("detached session connection input lane", () => {
  it("rejects old clipboard timestamps and explicit epochs before the backend", async () => {
    const invoke = vi.fn().mockResolvedValue(null);
    const input = new TerminalSessionInput(invoke, session("connected", "connection-a"));
    const old = input.capture()!;
    input.update(session("connected", "connection-b"));
    await expect(input.dispatch("serial", "old clipboard\r", "atomic", { awaitWrite: true, expectedConnectedSince: old.connectedSince })).rejects.toThrow();
    await expect(input.dispatch("serial", "old queued\r", "atomic", { awaitWrite: true, expectedInputEpoch: old.epoch })).rejects.toThrow();
    expect(invoke).not.toHaveBeenCalled();
    await input.dispatch("serial", "new clipboard\r", "atomic", { awaitWrite: true, expectedConnectedSince: "connection-b" });
    expect(invoke).toHaveBeenCalledExactlyOnceWith("send_text", expect.objectContaining({ text: "new clipboard\r", awaitWrite: true }));
  });

  it("resets a busy lane on reconnect and rejects both queued input and late acknowledgements", async () => {
    const ack = deferred<void>();
    const calls: Call[] = [];
    const onError = vi.fn(), onSuccess = vi.fn();
    const input = new TerminalSessionInput((async (command: string, args: Record<string, unknown>) => {
      calls.push({ command, args });
      return args.text === "old in flight\r" ? ack.promise : null;
    }) as never, session("connected", "connection-a"), { onError, onSuccess });
    const first = input.dispatch("serial", "old in flight\r", "atomic", { awaitWrite: true })!;
    const firstRejected = expect(first).rejects.toThrow();
    const queued = input.dispatch("serial", "old pending\r", "atomic", { awaitWrite: true })!;
    const queuedRejected = expect(queued).rejects.toThrow();
    input.update(session("disconnected"));
    input.update(session("connected", "connection-b"));
    await queuedRejected;
    await input.dispatch("serial", "new connection\r", "atomic", { awaitWrite: true });
    expect(calls.map(call => call.args.text)).toEqual(["old in flight\r", "new connection\r"]);
    ack.resolve();
    await firstRejected;
    expect(onError).not.toHaveBeenCalled();
    expect(onSuccess).toHaveBeenCalledTimes(1);
  });

  it("closes a pending native stream binding without replaying its keys after reconnect", async () => {
    const oldBind = deferred<{ streamId: string }>();
    const calls: Call[] = [];
    let begins = 0;
    const input = new TerminalSessionInput((async (command: string, args: Record<string, unknown>) => {
      calls.push({ command, args });
      if (command === "begin_terminal_input_stream") return ++begins === 1 ? oldBind.promise : { streamId: "new-stream" };
      return null;
    }) as never, session("connected", "connection-a"));
    input.dispatch("serial", "old keys", "interactive");
    await vi.waitFor(() => expect(begins).toBe(1));
    input.update(session("connected", "connection-b"));
    input.dispatch("serial", "new keys", "interactive");
    oldBind.resolve({ streamId: "old-stream" });
    await vi.waitFor(() => expect(calls.some(call => call.command === "send_text")).toBe(true));
    expect(calls.filter(call => call.command === "send_text")).toEqual([
      { command: "send_text", args: expect.objectContaining({ text: "new keys", inputOrder: { streamId: "new-stream", sequence: 0 } }) },
    ]);
    expect(calls).toContainEqual({ command: "close_terminal_input_stream", args: { sessionId: "serial", streamId: "old-stream" } });
  });

  it("preserves keyboard sequence and admitted atomic, private, binary and Telnet input", async () => {
    const calls: Call[] = [];
    const input = new TerminalSessionInput((async (command: string, args: Record<string, unknown>) => {
      calls.push({ command, args });
      return command === "begin_terminal_input_stream" ? { streamId: "native" } : null;
    }) as never, session("connected", "connection-a", "telnet"));
    input.dispatch("serial", "keyboard", "interactive");
    await vi.waitFor(() => expect(calls.some(call => call.command === "send_text")).toBe(true));
    await input.dispatch("serial", "secret\r", "atomic", { sensitive: true, awaitWrite: true });
    await input.dispatch("serial", String.fromCharCode(0xff, 0x80), "atomic", { binary: true, awaitWrite: true });
    expect(calls.filter(call => call.command === "send_text").map(call => call.args)).toEqual([
      expect.objectContaining({ text: "keyboard", interactive: true, inputOrder: { streamId: "native", sequence: 0 } }),
      expect.objectContaining({ text: "secret\r\n", sensitive: true, awaitWrite: true }),
    ]);
    expect(calls).toContainEqual({ command: "send_bytes", args: { sessionId: "serial", bytes: [0xff, 0x80], queued: true, expectedConnectedSince: "connection-a" } });
  });

  it("handles same-timestamp disconnects and deletion, rejecting queued work without a new runtime", async () => {
    const invoke = vi.fn().mockResolvedValue(null);
    const input = new TerminalSessionInput(invoke, session("connected", "same-time"));
    const old = input.capture()!;
    input.update(session("disconnected"));
    input.update(session("connected", "same-time"));
    expect(input.isCurrent(old)).toBe(false);
    await expect(input.dispatch("serial", "old", "atomic", { expectedInputEpoch: old.epoch, awaitWrite: true })).rejects.toThrow();
    input.update();
    await expect(input.dispatch("serial", "deleted", "atomic", { awaitWrite: true })).rejects.toThrow();
    expect(invoke).not.toHaveBeenCalled();
  });

  it("keeps streams on unrelated profile refreshes and clears fast queued keys on disconnect", async () => {
    const calls: Call[] = [];
    const input = new TerminalSessionInput((async (command: string, args: Record<string, unknown>) => {
      calls.push({ command, args });
      return command === "begin_terminal_input_stream" ? { streamId: "native" } : null;
    }) as never, session("connected", "connection-a"));
    input.dispatch("serial", "first", "interactive");
    await vi.waitFor(() => expect(calls.filter(call => call.command === "send_text")).toHaveLength(1));
    input.update({ ...session("connected", "connection-a"), profile: { ...session("connected", "connection-a").profile, name: "renamed" } });
    input.dispatch("serial", "second", "interactive");
    await vi.waitFor(() => expect(calls.filter(call => call.command === "send_text")).toHaveLength(2));
    expect(calls.filter(call => call.command === "begin_terminal_input_stream")).toHaveLength(1);
    expect(calls.filter(call => call.command === "send_text")[1].args.inputOrder).toEqual({ streamId: "native", sequence: 1 });
    input.dispatch("serial", "never send", "interactive");
    input.update(session("disconnected"));
    expect(input.dispatch("serial", "disconnected key", "interactive")).toBeUndefined();
    await Promise.resolve();
    expect(calls.filter(call => call.command === "send_text")).toHaveLength(2);
  });

  it("retains rejected editor drafts by rejecting current transport failures", async () => {
    const onError = vi.fn(), onSuccess = vi.fn();
    const input = new TerminalSessionInput((async () => { throw new Error("transport denied"); }) as never,
      session("connected", "connection-a"), { onError, onSuccess });
    await expect(input.dispatch("serial", "editor command\r", "atomic", { awaitWrite: true })).rejects.toThrow("transport denied");
    expect(onError).toHaveBeenCalledTimes(1);
    expect(onSuccess).not.toHaveBeenCalled();
  });
});
