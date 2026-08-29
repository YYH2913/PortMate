import { describe, expect, it } from "vitest";
import { TerminalInputPump, TerminalInputPumpRegistry } from "./terminal-input-pump";

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((next) => { resolve = next; });
  return { promise, resolve };
}

describe("terminal input pump", () => {
  it("flushes the first interactive input after a short burst window", async () => {
    const calls: string[] = [];
    const pump = new TerminalInputPump((_sessionId, text) => { calls.push(text); });

    pump.enqueueFast("router", "a", "interactive");

    expect(calls).toEqual([]);
    await expect.poll(() => calls).toEqual(["a"]);
  });

  it("coalesces a printable burst into one IPC call", async () => {
    const calls: string[] = [];
    const pump = new TerminalInputPump((_sessionId, text) => {
      calls.push(text);
    });

    for (const text of ["a", "b", "c", "d", "e", "f"]) {
      pump.enqueueFast("router", text, "interactive");
    }

    expect(calls).toEqual([]);
    await expect.poll(() => calls).toEqual(["abcdef"]);
  });

  it("coalesces input queued behind an in-flight request", async () => {
    const first = deferred();
    const calls: string[] = [];
    const pump = new TerminalInputPump((_sessionId, text) => {
      calls.push(text);
      return calls.length === 1 ? first.promise : Promise.resolve();
    });

    pump.enqueueFast("router", "a", "interactive");
    await expect.poll(() => calls).toEqual(["a"]);
    pump.enqueueFast("router", "b", "interactive");
    pump.enqueueFast("router", "c", "interactive");
    expect(calls).toEqual(["a"]);

    first.resolve();
    await first.promise;
    await expect.poll(() => calls).toEqual(["a", "bc"]);
  });

  it("flushes printable input before an atomic boundary", async () => {
    const calls: Array<{ text: string; origin: string }> = [];
    const pump = new TerminalInputPump((_sessionId, text, origin) => {
      calls.push({ text, origin });
    });

    pump.enqueueFast("router", "a", "interactive");
    const atomic = pump.enqueue("router", "\r", "atomic");

    expect(calls).toEqual([{ text: "a", origin: "interactive" }]);
    await atomic;
    expect(calls).toEqual([
      { text: "a", origin: "interactive" },
      { text: "\r", origin: "atomic" },
    ]);
  });

  it("keeps an atomic boundary behind in-flight fast input", async () => {
    const first = deferred();
    const calls: Array<{ text: string; origin: string }> = [];
    const pump = new TerminalInputPump((_sessionId, text, origin) => {
      calls.push({ text, origin });
      return calls.length === 1 ? first.promise : Promise.resolve();
    });

    pump.enqueueFast("router", "a", "interactive");
    const atomic = pump.enqueue("router", "paste", "atomic");
    expect(calls).toEqual([{ text: "a", origin: "interactive" }]);

    first.resolve();
    await first.promise;
    await atomic;
    expect(calls).toEqual([
      { text: "a", origin: "interactive" },
      { text: "paste", origin: "atomic" },
    ]);
  });

  it("coalesces queued interactive input without crossing atomic boundaries", async () => {
    const first = deferred();
    const second = deferred();
    const third = deferred();
    const calls: Array<{ text: string; origin: string }> = [];
    const pump = new TerminalInputPump((_sessionId, text, origin) => {
      calls.push({ text, origin });
      if (calls.length === 1) return first.promise;
      if (calls.length === 2) return second.promise;
      if (calls.length === 3) return third.promise;
      return Promise.resolve();
    });

    pump.enqueue("router", "a", "interactive");
    pump.enqueue("router", "b", "interactive");
    pump.enqueue("router", "c", "interactive");
    pump.enqueue("router", "paste", "atomic");
    pump.enqueue("router", "d", "interactive");
    expect(calls).toEqual([{ text: "a", origin: "interactive" }]);

    first.resolve();
    await first.promise;
    await Promise.resolve();
    expect(calls).toEqual([
      { text: "a", origin: "interactive" },
      { text: "bc", origin: "interactive" },
    ]);

    second.resolve();
    await second.promise;
    await expect.poll(() => calls.length).toBe(3);
    expect(calls).toEqual([
      { text: "a", origin: "interactive" },
      { text: "bc", origin: "interactive" },
      { text: "paste", origin: "atomic" },
    ]);

    third.resolve();
    await third.promise;
    await expect.poll(() => calls.length).toBe(4);
    expect(calls).toEqual([
      { text: "a", origin: "interactive" },
      { text: "bc", origin: "interactive" },
      { text: "paste", origin: "atomic" },
      { text: "d", origin: "interactive" },
    ]);
  });

  it("drops stale queued input after a session reset", async () => {
    const first = deferred();
    const calls: string[] = [];
    const pump = new TerminalInputPump((_sessionId, text) => {
      calls.push(text);
      return first.promise;
    });

    pump.enqueue("old", "a", "interactive");
    pump.enqueue("old", "b", "interactive");
    pump.reset();
    first.resolve();
    await first.promise;
    await Promise.resolve();

    expect(calls).toEqual(["a"]);
  });

  it("does not overlap an in-flight request after reset", async () => {
    const first = deferred();
    const calls: string[] = [];
    const pump = new TerminalInputPump((_sessionId, text) => {
      calls.push(text);
      return calls.length === 1 ? first.promise : Promise.resolve();
    });

    pump.enqueue("old", "a", "interactive");
    pump.reset();
    pump.enqueue("new", "b", "interactive");
    expect(calls).toEqual(["a"]);

    first.resolve();
    await first.promise;
    await expect.poll(() => calls.length).toBe(2);
    expect(calls).toEqual(["a", "b"]);
  });

  it("resolves callers after their queued item is sent", async () => {
    const first = deferred();
    const calls: string[] = [];
    const pump = new TerminalInputPump((_sessionId, text) => {
      calls.push(text);
      return calls.length === 1 ? first.promise : Promise.resolve();
    });
    let firstDone = false;
    let secondDone = false;
    const firstRequest = pump.enqueue("router", "a", "interactive").then(() => { firstDone = true; });
    const secondRequest = pump.enqueue("router", "b", "atomic").then(() => { secondDone = true; });
    await Promise.resolve();
    expect(firstDone).toBe(false);
    expect(secondDone).toBe(false);
    first.resolve();
    await firstRequest;
    expect(firstDone).toBe(true);
    expect(secondDone).toBe(false);
    await secondRequest;
    expect(secondDone).toBe(true);
  });

  it("forwards write acknowledgement options without changing queue ordering", async () => {
    const calls: Array<{ text: string; options?: { awaitWrite?: boolean } }> = [];
    const pump = new TerminalInputPump((_sessionId, text, _origin, options) => {
      calls.push({ text, options });
    });

    await pump.enqueue("router", "paced", "atomic", { awaitWrite: true });
    await pump.enqueue("router", "paste", "atomic");

    expect(calls).toEqual([
      { text: "paced", options: { awaitWrite: true } },
      { text: "paste", options: undefined },
    ]);
  });

  it("propagates requested write failures while keeping ordinary input fail-soft", async () => {
    const pump = new TerminalInputPump(async (_sessionId, _text, _origin, options) => {
      if (options?.awaitWrite) throw new Error("write failed");
      throw new Error("ordinary input failure");
    });

    await expect(pump.enqueue("router", "paced", "atomic", { awaitWrite: true }))
      .rejects.toThrow("write failed");
    await expect(pump.enqueue("router", "ordinary", "atomic"))
      .resolves.toBeUndefined();
  });

  it("runs independent registry sessions concurrently", async () => {
    const first = deferred();
    const calls: string[] = [];
    const registry = new TerminalInputPumpRegistry((sessionId) => {
      calls.push(sessionId);
      return sessionId === "slow" ? first.promise : Promise.resolve();
    });
    const slow = registry.enqueue("slow", "a", "interactive");
    const fast = registry.enqueue("fast", "b", "interactive");
    await fast;
    expect(calls).toEqual(["slow", "fast"]);
    first.resolve();
    await slow;
  });

  it("keeps synchronized printable input ahead of its atomic boundary", async () => {
    const calls: Array<{ text: string; origin: string }> = [];
    const registry = new TerminalInputPumpRegistry((_sessionId, text, origin) => {
      calls.push({ text, origin });
    });

    registry.dispatch("router", "a", "interactive");
    registry.dispatch("router", "b", "interactive");
    const enter = registry.dispatch("router", "\r", "atomic");

    expect(calls).toEqual([{ text: "ab", origin: "interactive" }]);
    await enter;
    expect(calls).toEqual([
      { text: "ab", origin: "interactive" },
      { text: "\r", origin: "atomic" },
    ]);
  });

  it("does not coalesce private and ordinary printable input", async () => {
    const calls: Array<{ text: string; sensitive: boolean }> = [];
    const registry = new TerminalInputPumpRegistry((_sessionId, text, _origin, options) => {
      calls.push({ text, sensitive: Boolean(options?.sensitive) });
    });

    registry.dispatch("router", "secret", "interactive", { sensitive: true });
    registry.dispatch("router", "visible", "interactive");

    await expect.poll(() => calls).toEqual([
      { text: "secret", sensitive: true },
      { text: "visible", sensitive: false },
    ]);
  });

  it("rejects an acknowledged write cancelled by a session reset", async () => {
    const first = deferred();
    const registry = new TerminalInputPumpRegistry((_sessionId, text) => (
      text === "active" ? first.promise : Promise.resolve()
    ));
    registry.dispatch("router", "active", "atomic");
    const queued = registry.dispatch("router", "paced", "atomic", { awaitWrite: true });

    registry.reset("router");

    await expect(queued).rejects.toThrow("cancelled before the transport write");
    first.resolve();
    await first.promise;
  });

  it("starts a fresh registry pump immediately after a session reset", async () => {
    const oldRequest = deferred();
    const calls: string[] = [];
    const registry = new TerminalInputPumpRegistry((_sessionId, text) => {
      calls.push(text);
      return calls.length === 1 ? oldRequest.promise : Promise.resolve();
    });

    registry.enqueueFast("router", "old", "interactive");
    await expect.poll(() => calls).toEqual(["old"]);
    registry.reset("router");
    registry.enqueueFast("router", "new", "interactive");

    expect(calls).toEqual(["old"]);
    await expect.poll(() => calls).toEqual(["old", "new"]);
    oldRequest.resolve();
    await oldRequest.promise;
  });
});
