import { describe, expect, it } from "vitest";
import { TerminalInputPump, TerminalInputPumpRegistry } from "./terminal-input-pump";

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((next) => { resolve = next; });
  return { promise, resolve };
}

describe("terminal input pump", () => {
  it("starts the first interactive input synchronously", () => {
    const calls: string[] = [];
    const pump = new TerminalInputPump((_sessionId, text) => { calls.push(text); });

    pump.enqueue("router", "a", "interactive");

    expect(calls).toEqual(["a"]);
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
});
