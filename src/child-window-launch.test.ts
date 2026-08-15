import { afterEach, describe, expect, it, vi } from "vitest";
import type { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { waitForChildWindowReady } from "./child-window-launch";

describe("child window launch", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("resolves after initialization without destroying the child", async () => {
    const child = fakeChildWindow();
    const initialize = vi.fn().mockResolvedValue(undefined);
    const ready = waitForChildWindowReady(child.window, initialize, "timed out");

    child.emit("tauri://created", undefined);
    await ready;

    expect(initialize).toHaveBeenCalledTimes(1);
    expect(child.destroy).not.toHaveBeenCalled();
    expect(child.listenerCount()).toBe(0);
  });

  it("destroys a created child when initialization fails", async () => {
    const child = fakeChildWindow();
    const ready = waitForChildWindowReady(
      child.window,
      () => Promise.reject(new Error("geometry failed")),
      "timed out",
    );

    child.emit("tauri://created", undefined);

    await expect(ready).rejects.toThrow("geometry failed");
    expect(child.destroy).toHaveBeenCalledTimes(1);
  });

  it("does not destroy an existing window when native creation rejects a duplicate label", async () => {
    const child = fakeChildWindow();
    const initialize = vi.fn().mockResolvedValue(undefined);
    const ready = waitForChildWindowReady(child.window, initialize, "timed out");

    child.emit("tauri://error", "window label already exists");

    await expect(ready).rejects.toThrow("window label already exists");
    expect(initialize).not.toHaveBeenCalled();
    expect(child.destroy).not.toHaveBeenCalled();
  });

  it("destroys a child that is created after the launch timeout", async () => {
    vi.useFakeTimers();
    const child = fakeChildWindow();
    const initialize = vi.fn().mockResolvedValue(undefined);
    const ready = waitForChildWindowReady(child.window, initialize, "creation timed out", 50);
    const rejected = expect(ready).rejects.toThrow("creation timed out");

    await vi.advanceTimersByTimeAsync(50);
    await rejected;
    child.emit("tauri://created", undefined);
    await Promise.resolve();

    expect(initialize).not.toHaveBeenCalled();
    expect(child.destroy).toHaveBeenCalledTimes(1);
  });

  it("destroys a created child when initialization exceeds the timeout", async () => {
    vi.useFakeTimers();
    const child = fakeChildWindow();
    const initialization = deferred<void>();
    const ready = waitForChildWindowReady(child.window, () => initialization.promise, "initialization timed out", 50);
    const rejected = expect(ready).rejects.toThrow("initialization timed out");

    child.emit("tauri://created", undefined);
    await vi.advanceTimersByTimeAsync(50);
    await rejected;

    expect(child.destroy).toHaveBeenCalledTimes(1);
    initialization.resolve();
    await Promise.resolve();
    expect(child.destroy).toHaveBeenCalledTimes(1);
  });
});

function fakeChildWindow() {
  const listeners = new Map<string, Set<(event: { payload: unknown }) => void>>();
  const destroy = vi.fn().mockResolvedValue(undefined);
  const window = {
    destroy,
    once: vi.fn(async (event: string, handler: (event: { payload: unknown }) => void) => {
      const handlers = listeners.get(event) ?? new Set();
      handlers.add(handler);
      listeners.set(event, handlers);
      return () => handlers.delete(handler);
    }),
  } as unknown as Pick<WebviewWindow, "destroy" | "once">;
  return {
    window,
    destroy,
    emit(event: string, payload: unknown) {
      for (const handler of [...(listeners.get(event) ?? [])]) handler({ payload });
    },
    listenerCount() {
      return [...listeners.values()].reduce((count, handlers) => count + handlers.size, 0);
    },
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}
