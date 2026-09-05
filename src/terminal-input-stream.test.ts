import { describe, expect, it, vi } from "vitest";
import { TerminalInputStreams } from "./terminal-input-stream";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

describe("native terminal input stream bindings", () => {
  it("shares one handshake and assigns sequence numbers before responses arrive", async () => {
    const ready = deferred<{ streamId: string }>();
    const invoke = vi.fn(async (command: string) => command === "begin_terminal_input_stream" ? ready.promise : null);
    const streams = new TerminalInputStreams(invoke as never);
    const first = streams.prepare("router", 0);
    const second = streams.prepare("router", 0);
    await Promise.resolve();
    ready.resolve({ streamId: "native-1" });
    expect(await first).toEqual({ streamId: "native-1", sequence: 0 });
    expect(await second).toEqual({ streamId: "native-1", sequence: 1 });
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("fences a pending bind on reset and serializes the replacement bind", async () => {
    const old = deferred<{ streamId: string }>();
    let begins = 0;
    const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
    const streams = new TerminalInputStreams((async (command: string, args: Record<string, unknown>) => {
      calls.push({ command, args });
      if (command === "begin_terminal_input_stream") return ++begins === 1 ? old.promise : { streamId: "new" };
      return null;
    }) as never);
    const first = streams.prepare("router", 0);
    const rejected = expect(first).rejects.toThrow("取消");
    streams.reset("router");
    const second = streams.prepare("router", 1);
    await vi.waitFor(() => expect(begins).toBe(1));
    old.resolve({ streamId: "old" });
    await rejected;
    expect(await second).toEqual({ streamId: "new", sequence: 0 });
    expect(calls.some(call => call.command === "close_terminal_input_stream" && call.args.streamId === "old")).toBe(true);
    streams.invalidate("router", { streamId: "old", sequence: 0 });
    expect(await streams.prepare("router", 1)).toEqual({ streamId: "new", sequence: 1 });
  });

  it("can recover from a failed bind without replaying old keystrokes", async () => {
    let begins = 0;
    const streams = new TerminalInputStreams((async (command: string) => {
      if (command !== "begin_terminal_input_stream") return null;
      if (++begins === 1) throw new Error("disconnected");
      return { streamId: "reconnected" };
    }) as never);
    await expect(streams.prepare("router", 0)).rejects.toThrow("disconnected");
    expect(await streams.prepare("router", 0)).toEqual({ streamId: "reconnected", sequence: 0 });
  });
});
