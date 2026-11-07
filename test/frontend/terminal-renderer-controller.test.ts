import { describe, expect, it, vi } from "vitest";
import { TerminalRendererController } from "../../src/terminal-renderer-controller";
import type { TerminalWebglContextInfo } from "../../src/terminal-renderer";

function fixture() {
  const listeners = new Set<() => void>();
  const addon = {
    dispose: vi.fn(),
    onContextLoss: vi.fn((listener: () => void) => {
      listeners.add(listener);
      return { dispose: vi.fn(() => { listeners.delete(listener); }) };
    }),
  };
  const context: TerminalWebglContextInfo = { support: "accelerated", renderer: "Intel Iris Xe", vendor: "Intel" };
  const options = {
    load: vi.fn(async () => () => addon),
    activate: vi.fn(),
    inspect: vi.fn(() => context),
    publish: vi.fn(),
    disabledReason: "disabled-transparency",
  };
  const controller = new TerminalRendererController(options);
  return { controller, options, addon, listeners, context };
}

describe("hardware renderer lifecycle", () => {
  it("coalesces pending imports and checks the real context after activation", async () => {
    const f = fixture();
    const first = f.controller.configure(true);
    expect(f.controller.configure(true)).toBe(first);
    await first;
    expect(f.options.load).toHaveBeenCalledOnce();
    expect(f.options.activate).toHaveBeenCalledExactlyOnceWith(f.addon);
    expect(f.options.inspect).toHaveBeenCalledOnce();
    expect(f.options.publish).toHaveBeenLastCalledWith({ renderer: "webgl", reason: "active", context: f.context });
    await f.controller.configure(true);
    expect(f.options.load).toHaveBeenCalledOnce();
    f.controller.dispose();
    expect(f.addon.dispose).toHaveBeenCalledOnce();
    expect(f.listeners.size).toBe(0);
  });

  it("rejects software found in the actual addon context", async () => {
    const f = fixture();
    f.options.inspect.mockReturnValue({ support: "software", renderer: "SwiftShader", vendor: "Google" });
    await f.controller.configure(true);
    expect(f.options.publish).toHaveBeenLastCalledWith({ renderer: "dom", reason: "disabled-software", context: f.options.inspect.mock.results[0].value });
    expect(f.addon.dispose).toHaveBeenCalledOnce();
    expect(f.listeners.size).toBe(0);
    await f.controller.configure(true);
    expect(f.options.load).toHaveBeenCalledOnce(); // No retry loop on every font/theme update.
  });

  it("recovers after an explicit disable/enable without caching a software rejection forever", async () => {
    const f = fixture();
    f.options.inspect.mockReturnValue({ support: "software", renderer: "SwiftShader", vendor: "Google" });
    await f.controller.configure(true);
    expect(f.addon.dispose).toHaveBeenCalledOnce();
    await f.controller.configure(false);
    f.options.inspect.mockReturnValue(f.context);
    await f.controller.configure(true);
    expect(f.options.activate).toHaveBeenCalledTimes(2);
    expect(f.options.publish).toHaveBeenLastCalledWith({ renderer: "webgl", reason: "active", context: f.context });
    f.controller.dispose();
  });

  it("cleans a partially activated addon when initialization throws", async () => {
    const f = fixture();
    f.options.activate.mockImplementation(() => { throw Error("GPU allocation failed"); });
    await f.controller.configure(true);
    expect(f.addon.dispose).toHaveBeenCalledOnce();
    expect(f.listeners.size).toBe(0);
    expect(f.options.publish).toHaveBeenLastCalledWith({ renderer: "dom", reason: "fallback-initialization" });
    f.controller.dispose();
    expect(f.addon.dispose).toHaveBeenCalledOnce();
  });

  it("keeps old imports from replacing a disabled or disposed view", async () => {
    for (const action of ["disable", "dispose"]) {
      const f = fixture();
      let resolve!: (factory: () => typeof f.addon) => void;
      f.options.load.mockReturnValue(new Promise(done => { resolve = done; }));
      const opening = f.controller.configure(true);
      if (action === "disable") await f.controller.configure(false);
      else f.controller.dispose();
      resolve(() => f.addon);
      await opening;
      expect(f.options.activate).not.toHaveBeenCalled();
      expect(f.options.inspect).not.toHaveBeenCalled();
      f.controller.dispose();
    }
  });

  it("falls back on context loss, ignoring stale callbacks after a later enable", async () => {
    const f = fixture();
    await f.controller.configure(true);
    const oldLoss = [...f.listeners][0];
    oldLoss();
    expect(f.addon.dispose).toHaveBeenCalledOnce();
    expect(f.options.publish).toHaveBeenLastCalledWith({ renderer: "dom", reason: "fallback-context-lost" });
    await f.controller.configure(true);
    expect(f.options.load).toHaveBeenCalledOnce();
    await f.controller.configure(false);
    await f.controller.configure(true);
    const calls = f.addon.dispose.mock.calls.length;
    oldLoss();
    expect(f.addon.dispose).toHaveBeenCalledTimes(calls);
    expect(f.options.publish).toHaveBeenLastCalledWith({ renderer: "webgl", reason: "active", context: f.context });
    f.controller.dispose();
  });

  it("handles loss during activation and releases resources even when cleanup throws", async () => {
    const f = fixture();
    f.options.activate.mockImplementation(() => { for (const listener of f.listeners) listener(); });
    f.addon.dispose.mockImplementation(() => { throw Error("driver lost"); });
    await f.controller.configure(true);
    expect(f.addon.dispose).toHaveBeenCalledOnce();
    expect(f.options.publish).toHaveBeenLastCalledWith({ renderer: "dom", reason: "fallback-context-lost", context: f.context });
    expect(f.listeners.size).toBe(0);
  });
});
