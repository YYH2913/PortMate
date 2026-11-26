import { TauriEvent } from "@tauri-apps/api/event";
import type { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { afterEach, describe, expect, it, vi } from "vitest";
import { t } from "../../src/i18n";
import { loadWindowGeometry } from "../../src/window-geometry-state";

const tauriWindow = vi.hoisted(() => ({
  availableMonitors: vi.fn(),
  getCurrentWindow: vi.fn(),
}));

vi.mock("@tauri-apps/api/window", () => {
  class PhysicalPosition {
    constructor(public x: number, public y: number) {}
  }
  class PhysicalSize {
    constructor(public width: number, public height: number) {}
  }
  return {
    availableMonitors: tauriWindow.availableMonitors,
    getCurrentWindow: tauriWindow.getCurrentWindow,
    PhysicalPosition,
    PhysicalSize,
  };
});

import { placeAndTrackChildWindow } from "../../src/window-geometry";

function stubBrowserWindow() {
  const values = new Map<string, string>();
  const setItem = vi.fn((key: string, value: string) => {
    values.set(key, value);
  });
  vi.stubGlobal("window", {
    localStorage: {
      getItem(key: string) {
        return values.get(key) ?? null;
      },
      setItem,
    },
    setTimeout: globalThis.setTimeout.bind(globalThis),
    clearTimeout: globalThis.clearTimeout.bind(globalThis),
  });
  tauriWindow.availableMonitors.mockResolvedValue([
    {
      workArea: {
        position: { x: 0, y: 0 },
        size: { width: 4_000, height: 3_000 },
      },
    },
  ]);
  tauriWindow.getCurrentWindow.mockReturnValue({
    outerPosition: vi.fn().mockResolvedValue({ x: 100, y: 80 }),
    scaleFactor: vi.fn().mockResolvedValue(1),
  });
  return { values, setItem };
}

async function flushMicrotasks(count = 30) {
  for (let i = 0; i < count; i += 1) {
    await Promise.resolve();
  }
}

describe("child window geometry", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.clearAllMocks();
  });

  it("persists final physical geometry on destroy without intercepting close requests", async () => {
    stubBrowserWindow();
    tauriWindow.getCurrentWindow.mockReturnValue({
      outerPosition: vi.fn().mockResolvedValue({ x: 100, y: 80 }),
      scaleFactor: vi.fn().mockResolvedValue(2),
    });

    const callOrder: string[] = [];
    const registeredEvents: string[] = [];
    const listeners = new Map<string, (event?: { payload?: unknown }) => void>();
    const unlistens = new Map<string, ReturnType<typeof vi.fn>>();
    const onCloseRequested = vi.fn();

    const child = {
      scaleFactor: vi.fn().mockResolvedValue(2),
      setSize: vi.fn().mockResolvedValue(undefined),
      setPosition: vi.fn().mockResolvedValue(undefined),
      show: vi.fn(async () => {
        callOrder.push("show");
      }),
      isMinimized: vi.fn().mockResolvedValue(false),
      outerPosition: vi.fn().mockResolvedValue({ x: 420, y: 360 }),
      innerSize: vi.fn().mockResolvedValue({ width: 1_400, height: 900 }),
      onCloseRequested,
      listen: vi.fn(async (event: string, listener: (e?: { payload?: unknown }) => void) => {
        registeredEvents.push(event);
        if (event === TauriEvent.WINDOW_DESTROYED) callOrder.push("listen:destroyed");
        listeners.set(event, listener);
        const unlisten = vi.fn().mockResolvedValue(undefined);
        unlistens.set(event, unlisten);
        return unlisten;
      }),
    } as unknown as WebviewWindow;

    await placeAndTrackChildWindow(child, {
      storageKey: "detached-pane:view-a",
      width: 960,
      height: 680,
      minWidth: 640,
      minHeight: 400,
    });
    await vi.waitFor(() => expect(child.outerPosition).toHaveBeenCalledTimes(1));

    expect(onCloseRequested).not.toHaveBeenCalled();
    expect(registeredEvents).not.toContain(TauriEvent.WINDOW_CLOSE_REQUESTED);
    expect(callOrder).toEqual(["listen:destroyed", "show"]);

    listeners.get(TauriEvent.WINDOW_MOVED)?.({ payload: { x: 1_024, y: 768 } });
    listeners.get(TauriEvent.WINDOW_RESIZED)?.({ payload: { width: 1_600, height: 1_000 } });
    // Simulated Windows minimize sentinel + zero-size resize must not overwrite normal bounds.
    listeners.get(TauriEvent.WINDOW_MOVED)?.({ payload: { x: -32_000, y: -32_000 } });
    listeners.get(TauriEvent.WINDOW_RESIZED)?.({ payload: { width: 0, height: 0 } });
    listeners.get(TauriEvent.WINDOW_DESTROYED)?.();

    expect(loadWindowGeometry(window.localStorage, "detached-pane:view-a", {
      minWidth: 1_280,
      minHeight: 800,
    })).toEqual({ x: 1_024, y: 768, width: 1_600, height: 1_000 });
    expect(unlistens.get(TauriEvent.WINDOW_DESTROYED)).toHaveBeenCalledTimes(1);
    expect(unlistens.get(TauriEvent.WINDOW_MOVED)).toHaveBeenCalledTimes(1);
    expect(unlistens.get(TauriEvent.WINDOW_RESIZED)).toHaveBeenCalledTimes(1);
    expect(unlistens.get(TauriEvent.WINDOW_SCALE_FACTOR_CHANGED)).toHaveBeenCalledTimes(1);
  });

  it("orders overlapping queries independently of generation changes", async () => {
    vi.useFakeTimers();
    stubBrowserWindow();

    const firstPosition = deferred<{ x: number; y: number }>();
    const firstSize = deferred<{ width: number; height: number }>();
    const secondPosition = deferred<{ x: number; y: number }>();
    const secondSize = deferred<{ width: number; height: number }>();
    let queryCall = 0;
    const listeners = new Map<string, (event?: { payload?: unknown }) => void>();

    const child = {
      scaleFactor: vi.fn().mockResolvedValue(1),
      setSize: vi.fn().mockResolvedValue(undefined),
      setPosition: vi.fn().mockResolvedValue(undefined),
      show: vi.fn().mockResolvedValue(undefined),
      isMinimized: vi.fn().mockResolvedValue(false),
      outerPosition: vi.fn(() => {
        queryCall += 1;
        return queryCall === 1 ? firstPosition.promise : secondPosition.promise;
      }),
      innerSize: vi.fn(() => (queryCall === 1 ? firstSize.promise : secondSize.promise)),
      listen: vi.fn(async (event: string, listener: (e?: { payload?: unknown }) => void) => {
        listeners.set(event, listener);
        return () => {};
      }),
    } as unknown as WebviewWindow;

    await placeAndTrackChildWindow(child, {
      storageKey: "detached-pane:view-ordered",
      width: 960,
      height: 680,
      minWidth: 640,
      minHeight: 400,
    });
    await flushMicrotasks();
    expect(child.outerPosition).toHaveBeenCalledTimes(1);

    // Payload-less resize invalidates size only, leaving positionGeneration equal
    // across query #1 and query #2 to independently verify appliedPositionQuery ordering.
    listeners.get(TauriEvent.WINDOW_RESIZED)?.({});
    await vi.advanceTimersByTimeAsync(200);
    expect(child.outerPosition).toHaveBeenCalledTimes(2);

    // Resolve the newer query FIRST, then the older query SECOND.
    secondPosition.resolve({ x: 800, y: 500 });
    secondSize.resolve({ width: 1_100, height: 740 });
    await flushMicrotasks();
    firstPosition.resolve({ x: 300, y: 200 });
    firstSize.resolve({ width: 960, height: 680 });
    await flushMicrotasks();
    listeners.get(TauriEvent.WINDOW_DESTROYED)?.();

    expect(loadWindowGeometry(window.localStorage, "detached-pane:view-ordered", {
      minWidth: 640,
      minHeight: 400,
    })).toEqual({ x: 800, y: 500, width: 1_100, height: 740 });
  });

  it("treats rejected isMinimized checks as unknown and protects saved bounds once minimized", async () => {
    vi.useFakeTimers();
    stubBrowserWindow();

    let minimizeMode: "normal" | "reject" | "minimized" = "normal";
    const listeners = new Map<string, (event?: { payload?: unknown }) => void>();
    const child = {
      scaleFactor: vi.fn().mockResolvedValue(1),
      setSize: vi.fn().mockResolvedValue(undefined),
      setPosition: vi.fn().mockResolvedValue(undefined),
      show: vi.fn().mockResolvedValue(undefined),
      isMinimized: vi.fn(async () => {
        if (minimizeMode === "reject") throw new Error("minimized IPC failed");
        return minimizeMode === "minimized";
      }),
      outerPosition: vi.fn().mockResolvedValue({ x: 500, y: 300 }),
      innerSize: vi.fn().mockResolvedValue({ width: 1_000, height: 700 }),
      listen: vi.fn(async (event: string, listener: (e?: { payload?: unknown }) => void) => {
        listeners.set(event, listener);
        return () => {};
      }),
    } as unknown as WebviewWindow;

    await placeAndTrackChildWindow(child, {
      storageKey: "detached-pane:view-min",
      width: 1_000,
      height: 700,
      minWidth: 640,
      minHeight: 400,
    });
    await flushMicrotasks();
    expect(loadWindowGeometry(window.localStorage, "detached-pane:view-min", {
      minWidth: 640,
      minHeight: 400,
    })).toEqual({ x: 500, y: 300, width: 1_000, height: 700 });

    // Rejected isMinimized() must mark state unknown and block BOTH debounced and destroy-time persistence.
    minimizeMode = "reject";
    listeners.get(TauriEvent.WINDOW_MOVED)?.({ payload: { x: 25, y: 25 } });
    await vi.advanceTimersByTimeAsync(200);
    expect(loadWindowGeometry(window.localStorage, "detached-pane:view-min", {
      minWidth: 640,
      minHeight: 400,
    })).toEqual({ x: 500, y: 300, width: 1_000, height: 700 });
    listeners.get(TauriEvent.WINDOW_DESTROYED)?.();
    expect(loadWindowGeometry(window.localStorage, "detached-pane:view-min", {
      minWidth: 640,
      minHeight: 400,
    })).toEqual({ x: 500, y: 300, width: 1_000, height: 700 });
  });

  it("blocks immediate destroy after confirmed minimization and never saves mixed scale bounds after rollback", async () => {
    vi.useFakeTimers();
    stubBrowserWindow();

    // Case A: Stale "normal" reply after minimize rollback + dirty move must NOT clear minimizeState.
    {
      const stalledAfterMinimize = deferred<boolean>();
      let minimizeCall = 0;
      let queryCall = 0;
      const listeners = new Map<string, (event?: { payload?: unknown }) => void>();

      const child = {
        scaleFactor: vi.fn().mockResolvedValue(1),
        setSize: vi.fn().mockResolvedValue(undefined),
        setPosition: vi.fn().mockResolvedValue(undefined),
        show: vi.fn().mockResolvedValue(undefined),
        isMinimized: vi.fn(() => {
          minimizeCall += 1;
          if (minimizeCall === 4) return stalledAfterMinimize.promise;
          if (minimizeCall === 5) return Promise.resolve(true);
          return Promise.resolve(false);
        }),
        outerPosition: vi.fn(() => {
          queryCall += 1;
          return queryCall === 1
            ? Promise.resolve({ x: 400, y: 250 })
            : Promise.resolve({ x: 700, y: 400 });
        }),
        innerSize: vi.fn(() => (
          queryCall === 1
            ? Promise.resolve({ width: 1_000, height: 700 })
            : Promise.resolve({ width: 1_300, height: 850 })
        )),
        listen: vi.fn(async (event: string, listener: (e?: { payload?: unknown }) => void) => {
          listeners.set(event, listener);
          return () => {};
        }),
      } as unknown as WebviewWindow;

      await placeAndTrackChildWindow(child, {
        storageKey: "serial-analyzer:stale-normal-after-min",
        width: 1_000,
        height: 700,
        minWidth: 640,
        minHeight: 400,
      });
      await flushMicrotasks();

      // Payload-less resize starts Query 1 (without changing scaleEpoch) and stalls at afterState.
      listeners.get(TauriEvent.WINDOW_RESIZED)?.({});
      await vi.advanceTimersByTimeAsync(200);
      expect(minimizeCall).toBe(4);

      // Query 2 observes minimization and rolls back to A, setting minimizeState = "minimized".
      listeners.get(TauriEvent.WINDOW_MOVED)?.({ payload: { x: 20, y: 20 } });
      await vi.advanceTimersByTimeAsync(200);
      expect(minimizeCall).toBe(5);

      // Emit another minimized move marking dirty=true, then release stale afterState=false!
      listeners.get(TauriEvent.WINDOW_MOVED)?.({ payload: { x: 10, y: 10 } });
      stalledAfterMinimize.resolve(false);
      await flushMicrotasks();
      listeners.get(TauriEvent.WINDOW_DESTROYED)?.();

      expect(loadWindowGeometry(window.localStorage, "serial-analyzer:stale-normal-after-min", {
        minWidth: 640,
        minHeight: 400,
      })).toEqual({ x: 400, y: 250, width: 1_000, height: 700 });
    }

    // Case B: Resize before minimize, then restore emitting ONLY move -> debounce must query
    // fresh outerPosition + innerSize instead of saving the old saved size with the new move.
    {
      let minimized = false;
      let currentPos = { x: 400, y: 250 };
      let currentSize = { width: 1_000, height: 700 };
      const listeners = new Map<string, (event?: { payload?: unknown }) => void>();
      const child = {
        scaleFactor: vi.fn().mockResolvedValue(1),
        setSize: vi.fn().mockResolvedValue(undefined),
        setPosition: vi.fn().mockResolvedValue(undefined),
        show: vi.fn().mockResolvedValue(undefined),
        isMinimized: vi.fn(async () => minimized),
        outerPosition: vi.fn(async () => currentPos),
        innerSize: vi.fn(async () => currentSize),
        listen: vi.fn(async (event: string, listener: (e?: { payload?: unknown }) => void) => {
          listeners.set(event, listener);
          return () => {};
        }),
      } as unknown as WebviewWindow;

      await placeAndTrackChildWindow(child, {
        storageKey: "serial-analyzer:resize-min-restore-move",
        width: 1_000,
        height: 700,
        minWidth: 640,
        minHeight: 400,
      });
      await flushMicrotasks();

      // Resize before debounce, then minimize so debounce invalidates live bounds.
      listeners.get(TauriEvent.WINDOW_RESIZED)?.({ payload: { width: 1_240, height: 820 } });
      minimized = true;
      await vi.advanceTimersByTimeAsync(200);

      // Restore to a DISTINCT size (1320x860 vs pre-minimize 1240x820) emitting ONLY a move event
      // to prove capture() actually queried the restored size rather than reusing stale state.
      minimized = false;
      currentPos = { x: 920, y: 540 };
      currentSize = { width: 1_320, height: 860 };
      listeners.get(TauriEvent.WINDOW_MOVED)?.({ payload: { x: 920, y: 540 } });
      await vi.advanceTimersByTimeAsync(200);
      listeners.get(TauriEvent.WINDOW_DESTROYED)?.();

      expect(loadWindowGeometry(window.localStorage, "serial-analyzer:resize-min-restore-move", {
        minWidth: 640,
        minHeight: 400,
      })).toEqual({ x: 920, y: 540, width: 1_320, height: 860 });
    }

    // Case C: Positive-looking move emitted WHILE minimized must NOT survive into a restore
    // that emits only a resize; debounce must run a full capture() and save real restored bounds.
    {
      let minimized = false;
      let currentPos = { x: 400, y: 250 };
      let currentSize = { width: 1_000, height: 700 };
      const listeners = new Map<string, (event?: { payload?: unknown }) => void>();
      const child = {
        scaleFactor: vi.fn().mockResolvedValue(1),
        setSize: vi.fn().mockResolvedValue(undefined),
        setPosition: vi.fn().mockResolvedValue(undefined),
        show: vi.fn().mockResolvedValue(undefined),
        isMinimized: vi.fn(async () => minimized),
        outerPosition: vi.fn(async () => currentPos),
        innerSize: vi.fn(async () => currentSize),
        listen: vi.fn(async (event: string, listener: (e?: { payload?: unknown }) => void) => {
          listeners.set(event, listener);
          return () => {};
        }),
      } as unknown as WebviewWindow;

      await placeAndTrackChildWindow(child, {
        storageKey: "serial-analyzer:min-move-restore-resize",
        width: 1_000,
        height: 700,
        minWidth: 640,
        minHeight: 400,
      });
      await flushMicrotasks();

      // Confirm minimization.
      minimized = true;
      listeners.get(TauriEvent.WINDOW_MOVED)?.({ payload: { x: 12, y: 12 } });
      await vi.advanceTimersByTimeAsync(200);

      // Emit a positive-looking move while still minimized, then restore emitting ONLY resize.
      listeners.get(TauriEvent.WINDOW_MOVED)?.({ payload: { x: 18, y: 18 } });
      minimized = false;
      currentPos = { x: 620, y: 340 };
      currentSize = { width: 1_180, height: 790 };
      listeners.get(TauriEvent.WINDOW_RESIZED)?.({ payload: { width: 1_180, height: 790 } });
      await vi.advanceTimersByTimeAsync(200);
      listeners.get(TauriEvent.WINDOW_DESTROYED)?.();

      expect(loadWindowGeometry(window.localStorage, "serial-analyzer:min-move-restore-resize", {
        minWidth: 640,
        minHeight: 400,
      })).toEqual({ x: 620, y: 340, width: 1_180, height: 790 });
    }
  });

  it("tracks per-field freshness across scale changes and partial getter rejections", async () => {
    stubBrowserWindow();

    // Case A: Scale change followed by BOTH fresh move and resize saves immediately on destroy.
    {
      const listeners = new Map<string, (event?: { payload?: unknown }) => void>();
      const child = {
        scaleFactor: vi.fn().mockResolvedValue(1),
        setSize: vi.fn().mockResolvedValue(undefined),
        setPosition: vi.fn().mockResolvedValue(undefined),
        show: vi.fn().mockResolvedValue(undefined),
        isMinimized: vi.fn().mockResolvedValue(false),
        outerPosition: vi.fn().mockResolvedValue({ x: 200, y: 150 }),
        innerSize: vi.fn().mockResolvedValue({ width: 900, height: 650 }),
        listen: vi.fn(async (event: string, listener: (e?: { payload?: unknown }) => void) => {
          listeners.set(event, listener);
          return () => {};
        }),
      } as unknown as WebviewWindow;

      await placeAndTrackChildWindow(child, {
        storageKey: "serial-analyzer:scale-both",
        width: 900,
        height: 650,
        minWidth: 640,
        minHeight: 400,
      });
      await flushMicrotasks();

      listeners.get(TauriEvent.WINDOW_SCALE_FACTOR_CHANGED)?.();
      listeners.get(TauriEvent.WINDOW_MOVED)?.({ payload: { x: 880, y: 520 } });
      listeners.get(TauriEvent.WINDOW_RESIZED)?.({ payload: { width: 1_280, height: 860 } });
      listeners.get(TauriEvent.WINDOW_DESTROYED)?.();

      expect(loadWindowGeometry(window.localStorage, "serial-analyzer:scale-both", {
        minWidth: 640,
        minHeight: 400,
      })).toEqual({ x: 880, y: 520, width: 1_280, height: 860 });
    }

    // Case B: Scale change followed by ONLY a move must NOT save mixed pre-scale size.
    {
      const listeners = new Map<string, (event?: { payload?: unknown }) => void>();
      const child = {
        scaleFactor: vi.fn().mockResolvedValue(1),
        setSize: vi.fn().mockResolvedValue(undefined),
        setPosition: vi.fn().mockResolvedValue(undefined),
        show: vi.fn().mockResolvedValue(undefined),
        isMinimized: vi.fn().mockResolvedValue(false),
        outerPosition: vi.fn().mockResolvedValue({ x: 200, y: 150 }),
        innerSize: vi.fn().mockResolvedValue({ width: 900, height: 650 }),
        listen: vi.fn(async (event: string, listener: (e?: { payload?: unknown }) => void) => {
          listeners.set(event, listener);
          return () => {};
        }),
      } as unknown as WebviewWindow;

      await placeAndTrackChildWindow(child, {
        storageKey: "serial-analyzer:scale-half",
        width: 900,
        height: 650,
        minWidth: 640,
        minHeight: 400,
      });
      await flushMicrotasks();

      listeners.get(TauriEvent.WINDOW_SCALE_FACTOR_CHANGED)?.();
      listeners.get(TauriEvent.WINDOW_MOVED)?.({ payload: { x: 880, y: 520 } });
      listeners.get(TauriEvent.WINDOW_DESTROYED)?.();

      expect(loadWindowGeometry(window.localStorage, "serial-analyzer:scale-half", {
        minWidth: 640,
        minHeight: 400,
      })).toEqual({ x: 200, y: 150, width: 900, height: 650 });
    }

    // Case C: Partial getter rejection keeps the fulfilled field when the missing field arrives live.
    {
      const listeners = new Map<string, (event?: { payload?: unknown }) => void>();
      const child = {
        scaleFactor: vi.fn().mockResolvedValue(1),
        setSize: vi.fn().mockResolvedValue(undefined),
        setPosition: vi.fn().mockResolvedValue(undefined),
        show: vi.fn().mockResolvedValue(undefined),
        isMinimized: vi.fn().mockResolvedValue(false),
        outerPosition: vi.fn().mockResolvedValue({ x: 610, y: 310 }),
        innerSize: vi.fn().mockRejectedValue(new Error("size getter unavailable")),
        listen: vi.fn(async (event: string, listener: (e?: { payload?: unknown }) => void) => {
          listeners.set(event, listener);
          return () => {};
        }),
      } as unknown as WebviewWindow;

      await placeAndTrackChildWindow(child, {
        storageKey: "serial-analyzer:partial-getter",
        width: 900,
        height: 650,
        minWidth: 640,
        minHeight: 400,
      });
      await flushMicrotasks();

      listeners.get(TauriEvent.WINDOW_RESIZED)?.({ payload: { width: 1_120, height: 720 } });
      listeners.get(TauriEvent.WINDOW_DESTROYED)?.();

      expect(loadWindowGeometry(window.localStorage, "serial-analyzer:partial-getter", {
        minWidth: 640,
        minHeight: 400,
      })).toEqual({ x: 610, y: 310, width: 1_120, height: 720 });
    }
  });

  it("prevents an older stalled query from repopulating bounds after a minimize rollback", async () => {
    vi.useFakeTimers();
    stubBrowserWindow();

    const stalledAfterMinimize = deferred<boolean>();
    let minimizeCall = 0;
    let queryCall = 0;
    const listeners = new Map<string, (event?: { payload?: unknown }) => void>();

    const child = {
      scaleFactor: vi.fn().mockResolvedValue(1),
      setSize: vi.fn().mockResolvedValue(undefined),
      setPosition: vi.fn().mockResolvedValue(undefined),
      show: vi.fn().mockResolvedValue(undefined),
      isMinimized: vi.fn(() => {
        minimizeCall += 1;
        // 1,2: initial capture (normal)
        // 3: post-scale Query 1 beforeState (normal)
        // 4: post-scale Query 1 afterState (stalled -> resolves false AFTER rollback)
        // 5: Query 2 beforeState (minimized -> triggers rollback to A)
        if (minimizeCall === 4) return stalledAfterMinimize.promise;
        if (minimizeCall === 5) return Promise.resolve(true);
        return Promise.resolve(false);
      }),
      outerPosition: vi.fn(() => {
        queryCall += 1;
        return queryCall === 1
          ? Promise.resolve({ x: 400, y: 250 })
          : Promise.resolve({ x: 700, y: 400 });
      }),
      innerSize: vi.fn(() => (
        queryCall === 1
          ? Promise.resolve({ width: 1_000, height: 700 })
          : Promise.resolve({ width: 1_300, height: 850 })
      )),
      listen: vi.fn(async (event: string, listener: (e?: { payload?: unknown }) => void) => {
        listeners.set(event, listener);
        return () => {};
      }),
    } as unknown as WebviewWindow;

    await placeAndTrackChildWindow(child, {
      storageKey: "serial-analyzer:rollback-race",
      width: 1_000,
      height: 700,
      minWidth: 640,
      minHeight: 400,
    });
    await flushMicrotasks();

    // Scale change triggers Query 1, which stalls at afterState (minimizeCall === 4).
    listeners.get(TauriEvent.WINDOW_SCALE_FACTOR_CHANGED)?.();
    await vi.advanceTimersByTimeAsync(200);
    expect(minimizeCall).toBe(4);

    // Deliver a newer move and run debounce Query 2, which observes minimization and rolls back to A.
    listeners.get(TauriEvent.WINDOW_MOVED)?.({ payload: { x: 710, y: 410 } });
    await vi.advanceTimersByTimeAsync(200);
    expect(minimizeCall).toBe(5);

    // Release Query 1's stale afterState === false; it must NOT mix post-scale size with A's position.
    stalledAfterMinimize.resolve(false);
    await flushMicrotasks();
    listeners.get(TauriEvent.WINDOW_DESTROYED)?.();

    expect(loadWindowGeometry(window.localStorage, "serial-analyzer:rollback-race", {
      minWidth: 640,
      minHeight: 400,
    })).toEqual({ x: 400, y: 250, width: 1_000, height: 700 });
  });

  it("guards delayed minimize rollbacks against newer live bounds in both capture and debounced flush", async () => {
    vi.useFakeTimers();
    stubBrowserWindow();

    // Subcase A: Delayed minimize inside flushDebouncedGeometry()
    {
      const delayedFlushMinimize = deferred<boolean>();
      let minimizeCall = 0;
      const listeners = new Map<string, (event?: { payload?: unknown }) => void>();
      const child = {
        scaleFactor: vi.fn().mockResolvedValue(1),
        setSize: vi.fn().mockResolvedValue(undefined),
        setPosition: vi.fn().mockResolvedValue(undefined),
        show: vi.fn().mockResolvedValue(undefined),
        isMinimized: vi.fn(() => {
          minimizeCall += 1;
          if (minimizeCall === 3) return delayedFlushMinimize.promise;
          return Promise.resolve(false);
        }),
        outerPosition: vi.fn().mockResolvedValue({ x: 400, y: 250 }),
        innerSize: vi.fn().mockResolvedValue({ width: 1_000, height: 700 }),
        listen: vi.fn(async (event: string, listener: (e?: { payload?: unknown }) => void) => {
          listeners.set(event, listener);
          return () => {};
        }),
      } as unknown as WebviewWindow;

      await placeAndTrackChildWindow(child, {
        storageKey: "serial-analyzer:delayed-flush",
        width: 1_000,
        height: 700,
        minWidth: 640,
        minHeight: 400,
      });
      await flushMicrotasks();

      listeners.get(TauriEvent.WINDOW_MOVED)?.({ payload: { x: 500, y: 320 } });
      listeners.get(TauriEvent.WINDOW_RESIZED)?.({ payload: { width: 1_050, height: 720 } });
      await vi.advanceTimersByTimeAsync(200);
      expect(minimizeCall).toBe(3);

      // Newer live bounds distinct from getter values arrive BEFORE delayed minimize resolves true.
      listeners.get(TauriEvent.WINDOW_MOVED)?.({ payload: { x: 640, y: 420 } });
      listeners.get(TauriEvent.WINDOW_RESIZED)?.({ payload: { width: 1_100, height: 760 } });
      delayedFlushMinimize.resolve(true);
      await flushMicrotasks();

      // Destroy immediately (no replacement query) to prove the rollback guard preserved live bounds.
      listeners.get(TauriEvent.WINDOW_DESTROYED)?.();
      expect(loadWindowGeometry(window.localStorage, "serial-analyzer:delayed-flush", {
        minWidth: 640,
        minHeight: 400,
      })).toEqual({ x: 640, y: 420, width: 1_100, height: 760 });
    }

    // Subcase B: Delayed minimize inside capture()
    {
      const delayedCaptureMinimize = deferred<boolean>();
      const listeners = new Map<string, (event?: { payload?: unknown }) => void>();
      const child = {
        scaleFactor: vi.fn().mockResolvedValue(1),
        setSize: vi.fn().mockResolvedValue(undefined),
        setPosition: vi.fn().mockResolvedValue(undefined),
        show: vi.fn().mockResolvedValue(undefined),
        isMinimized: vi.fn(() => delayedCaptureMinimize.promise),
        outerPosition: vi.fn().mockResolvedValue({ x: 900, y: 500 }),
        innerSize: vi.fn().mockResolvedValue({ width: 1_200, height: 800 }),
        listen: vi.fn(async (event: string, listener: (e?: { payload?: unknown }) => void) => {
          listeners.set(event, listener);
          return () => {};
        }),
      } as unknown as WebviewWindow;

      await placeAndTrackChildWindow(child, {
        storageKey: "serial-analyzer:delayed-capture",
        width: 1_000,
        height: 700,
        minWidth: 640,
        minHeight: 400,
      });
      await flushMicrotasks();

      listeners.get(TauriEvent.WINDOW_MOVED)?.({ payload: { x: 640, y: 420 } });
      listeners.get(TauriEvent.WINDOW_RESIZED)?.({ payload: { width: 1_100, height: 760 } });
      delayedCaptureMinimize.resolve(true);
      await flushMicrotasks();

      listeners.get(TauriEvent.WINDOW_DESTROYED)?.();
      expect(loadWindowGeometry(window.localStorage, "serial-analyzer:delayed-capture", {
        minWidth: 640,
        minHeight: 400,
      })).toEqual({ x: 640, y: 420, width: 1_100, height: 760 });
    }

    // Subcase C: Delayed minimize at capture()'s SECOND check (afterState) must not roll back newer live bounds.
    {
      const delayedAfterState = deferred<boolean>();
      let minimizeCall = 0;
      const listeners = new Map<string, (event?: { payload?: unknown }) => void>();
      const child = {
        scaleFactor: vi.fn().mockResolvedValue(1),
        setSize: vi.fn().mockResolvedValue(undefined),
        setPosition: vi.fn().mockResolvedValue(undefined),
        show: vi.fn().mockResolvedValue(undefined),
        isMinimized: vi.fn(() => {
          minimizeCall += 1;
          return minimizeCall === 2 ? delayedAfterState.promise : Promise.resolve(false);
        }),
        outerPosition: vi.fn().mockResolvedValue({ x: 900, y: 500 }),
        innerSize: vi.fn().mockResolvedValue({ width: 1_200, height: 800 }),
        listen: vi.fn(async (event: string, listener: (e?: { payload?: unknown }) => void) => {
          listeners.set(event, listener);
          return () => {};
        }),
      } as unknown as WebviewWindow;

      await placeAndTrackChildWindow(child, {
        storageKey: "serial-analyzer:delayed-capture-after",
        width: 1_000,
        height: 700,
        minWidth: 640,
        minHeight: 400,
      });
      await flushMicrotasks();
      expect(minimizeCall).toBe(2);

      listeners.get(TauriEvent.WINDOW_MOVED)?.({ payload: { x: 650, y: 430 } });
      listeners.get(TauriEvent.WINDOW_RESIZED)?.({ payload: { width: 1_120, height: 770 } });
      delayedAfterState.resolve(true);
      await flushMicrotasks();

      listeners.get(TauriEvent.WINDOW_DESTROYED)?.();
      expect(loadWindowGeometry(window.localStorage, "serial-analyzer:delayed-capture-after", {
        minWidth: 640,
        minHeight: 400,
      })).toEqual({ x: 650, y: 430, width: 1_120, height: 770 });
    }
  });

  it("discards pre-show buffered events on launch abort and aborts launch if destroyed while arming", async () => {
    vi.useFakeTimers();
    const { setItem } = stubBrowserWindow();

    // Case A: beforeShow throws after move+resize arrived during arming -> zero writes & full cleanup.
    {
      const listeners = new Map<string, (event?: { payload?: unknown }) => void>();
      const rejectingUnlisten = vi.fn().mockRejectedValue(new Error("unlisten rejected"));
      const child = {
        scaleFactor: vi.fn().mockResolvedValue(1),
        setSize: vi.fn().mockResolvedValue(undefined),
        setPosition: vi.fn().mockResolvedValue(undefined),
        show: vi.fn().mockResolvedValue(undefined),
        listen: vi.fn(async (event: string, listener: (e?: { payload?: unknown }) => void) => {
          listeners.set(event, listener);
          return rejectingUnlisten;
        }),
      } as unknown as WebviewWindow;

      await expect(placeAndTrackChildWindow(child, {
        storageKey: "serial-analyzer:abort-before-show",
        width: 1_000,
        height: 700,
        minWidth: 720,
        minHeight: 480,
        beforeShow: () => {
          listeners.get(TauriEvent.WINDOW_MOVED)?.({ payload: { x: 500, y: 300 } });
          listeners.get(TauriEvent.WINDOW_RESIZED)?.({ payload: { width: 1_000, height: 700 } });
          throw new Error("stale launch");
        },
      })).rejects.toThrow("stale launch");

      expect(child.show).not.toHaveBeenCalled();
      expect(setItem).not.toHaveBeenCalled();
      expect(rejectingUnlisten).toHaveBeenCalledTimes(4);
    }

    // Case B: show() stalls past the 160ms debounce while move+resize arrive, then rejects ->
    // zero premature debounced writes and zero flush writes on rejection.
    {
      const listeners = new Map<string, (event?: { payload?: unknown }) => void>();
      const pendingShow = deferred<void>();
      const unlisten = vi.fn().mockResolvedValue(undefined);
      const child = {
        scaleFactor: vi.fn().mockResolvedValue(1),
        setSize: vi.fn().mockResolvedValue(undefined),
        setPosition: vi.fn().mockResolvedValue(undefined),
        show: vi.fn(() => pendingShow.promise),
        isMinimized: vi.fn().mockResolvedValue(false),
        listen: vi.fn(async (event: string, listener: (e?: { payload?: unknown }) => void) => {
          listeners.set(event, listener);
          return unlisten;
        }),
      } as unknown as WebviewWindow;

      const placement = placeAndTrackChildWindow(child, {
        storageKey: "serial-analyzer:show-reject",
        width: 1_000,
        height: 700,
        minWidth: 720,
        minHeight: 480,
      });
      await flushMicrotasks();
      expect(child.show).toHaveBeenCalledTimes(1);

      listeners.get(TauriEvent.WINDOW_MOVED)?.({ payload: { x: 500, y: 300 } });
      listeners.get(TauriEvent.WINDOW_RESIZED)?.({ payload: { width: 1_000, height: 700 } });
      await vi.advanceTimersByTimeAsync(250);
      expect(setItem).not.toHaveBeenCalled();

      const rejection = expect(placement).rejects.toThrow("show failed");
      pendingShow.reject(new Error("show failed"));
      await rejection;

      expect(setItem).not.toHaveBeenCalled();
      expect(unlisten).toHaveBeenCalledTimes(4);
    }

    // Case C: WINDOW_DESTROYED fires while its OWN listen() handle is still pending ->
    // immediately aborts before registering other listeners or calling beforeShow/show(),
    // and cleans up the late-resolving destroyed handle.
    {
      const pendingDestroyedReg = deferred<() => Promise<void>>();
      const lateDestroyedUnlisten = vi.fn().mockResolvedValue(undefined);
      const beforeShow = vi.fn();
      let destroyedListener: (() => void) | undefined;

      const child = {
        scaleFactor: vi.fn().mockResolvedValue(1),
        setSize: vi.fn().mockResolvedValue(undefined),
        setPosition: vi.fn().mockResolvedValue(undefined),
        show: vi.fn().mockResolvedValue(undefined),
        listen: vi.fn((event: string, listener: () => void) => {
          if (event === TauriEvent.WINDOW_DESTROYED) {
            destroyedListener = listener;
            return pendingDestroyedReg.promise;
          }
          return Promise.resolve(() => {});
        }),
      } as unknown as WebviewWindow;

      const placement = placeAndTrackChildWindow(child, {
        storageKey: "serial-analyzer:destroy-during-first-listen",
        width: 1_000,
        height: 700,
        minWidth: 720,
        minHeight: 480,
        beforeShow,
      });
      await flushMicrotasks();
      expect(destroyedListener).toBeDefined();

      destroyedListener!();
      await expect(placement).rejects.toThrow(t("child-window-closed-before-ready"));

      expect(beforeShow).not.toHaveBeenCalled();
      expect(child.show).not.toHaveBeenCalled();
      expect(child.listen).toHaveBeenCalledTimes(1);

      pendingDestroyedReg.resolve(lateDestroyedUnlisten);
      await flushMicrotasks();
      expect(lateDestroyedUnlisten).toHaveBeenCalledTimes(1);
    }

    // Case D: WINDOW_DESTROYED fires while WINDOW_RESIZED registration is still pending ->
    // aborts before beforeShow/show() and cleans up the late-resolving registration.
    {
      const pendingResizeReg = deferred<() => Promise<void>>();
      const lateUnlisten = vi.fn().mockResolvedValue(undefined);
      const immediateUnlisten = vi.fn().mockResolvedValue(undefined);
      const beforeShow = vi.fn();
      let destroyedListener: (() => void) | undefined;

      const child = {
        scaleFactor: vi.fn().mockResolvedValue(1),
        setSize: vi.fn().mockResolvedValue(undefined),
        setPosition: vi.fn().mockResolvedValue(undefined),
        show: vi.fn().mockResolvedValue(undefined),
        listen: vi.fn((event: string, listener: () => void) => {
          if (event === TauriEvent.WINDOW_DESTROYED) {
            destroyedListener = listener;
            return Promise.resolve(immediateUnlisten);
          }
          if (event === TauriEvent.WINDOW_RESIZED) {
            return pendingResizeReg.promise;
          }
          return Promise.resolve(immediateUnlisten);
        }),
      } as unknown as WebviewWindow;

      const placement = placeAndTrackChildWindow(child, {
        storageKey: "serial-analyzer:destroy-while-arming",
        width: 1_000,
        height: 700,
        minWidth: 720,
        minHeight: 480,
        beforeShow,
      });
      await flushMicrotasks();
      expect(destroyedListener).toBeDefined();

      destroyedListener!();
      await expect(placement).rejects.toThrow(t("child-window-closed-before-ready"));

      expect(beforeShow).not.toHaveBeenCalled();
      expect(child.show).not.toHaveBeenCalled();
      expect(immediateUnlisten).toHaveBeenCalled();
      expect(lateUnlisten).not.toHaveBeenCalled();

      pendingResizeReg.resolve(lateUnlisten);
      await flushMicrotasks();
      expect(lateUnlisten).toHaveBeenCalledTimes(1);
    }

    // Case E: Payload-less resize arriving AFTER valid bounds invalidates ONLY size,
    // preserving already-valid position when a subsequent live resize arrives before destroy.
    {
      const listeners = new Map<string, (event?: { payload?: unknown }) => void>();
      const child = {
        scaleFactor: vi.fn().mockResolvedValue(1),
        setSize: vi.fn().mockResolvedValue(undefined),
        setPosition: vi.fn().mockResolvedValue(undefined),
        show: vi.fn().mockResolvedValue(undefined),
        isMinimized: vi.fn().mockResolvedValue(false),
        outerPosition: vi.fn().mockResolvedValue({ x: 520, y: 280 }),
        innerSize: vi.fn().mockResolvedValue({ width: 960, height: 640 }),
        listen: vi.fn(async (event: string, listener: (e?: { payload?: unknown }) => void) => {
          listeners.set(event, listener);
          return () => {};
        }),
      } as unknown as WebviewWindow;

      await placeAndTrackChildWindow(child, {
        storageKey: "serial-analyzer:invalid-resize-keeps-pos",
        width: 960,
        height: 640,
        minWidth: 640,
        minHeight: 400,
      });
      await flushMicrotasks();

      // Move to a new valid position, then receive a payload-less resize (invalidates size only),
      // then receive a valid resize and destroy immediately (no query needed for position).
      listeners.get(TauriEvent.WINDOW_MOVED)?.({ payload: { x: 777, y: 333 } });
      listeners.get(TauriEvent.WINDOW_RESIZED)?.({});
      listeners.get(TauriEvent.WINDOW_RESIZED)?.({ payload: { width: 1_111, height: 722 } });
      listeners.get(TauriEvent.WINDOW_DESTROYED)?.();

      expect(loadWindowGeometry(window.localStorage, "serial-analyzer:invalid-resize-keeps-pos", {
        minWidth: 640,
        minHeight: 400,
      })).toEqual({ x: 777, y: 333, width: 1_111, height: 722 });
    }

    // Case F: Dirty move followed by payload-less resize and IMMEDIATE destroy (no replacement resize)
    // must NOT flush the new position paired with the invalidated old size.
    {
      const listeners = new Map<string, (event?: { payload?: unknown }) => void>();
      const child = {
        scaleFactor: vi.fn().mockResolvedValue(1),
        setSize: vi.fn().mockResolvedValue(undefined),
        setPosition: vi.fn().mockResolvedValue(undefined),
        show: vi.fn().mockResolvedValue(undefined),
        isMinimized: vi.fn().mockResolvedValue(false),
        outerPosition: vi.fn().mockResolvedValue({ x: 520, y: 280 }),
        innerSize: vi.fn().mockResolvedValue({ width: 960, height: 640 }),
        listen: vi.fn(async (event: string, listener: (e?: { payload?: unknown }) => void) => {
          listeners.set(event, listener);
          return () => {};
        }),
      } as unknown as WebviewWindow;

      await placeAndTrackChildWindow(child, {
        storageKey: "serial-analyzer:invalid-resize-immediate-destroy",
        width: 960,
        height: 640,
        minWidth: 640,
        minHeight: 400,
      });
      await flushMicrotasks();

      listeners.get(TauriEvent.WINDOW_MOVED)?.({ payload: { x: 777, y: 333 } });
      listeners.get(TauriEvent.WINDOW_RESIZED)?.({});
      listeners.get(TauriEvent.WINDOW_DESTROYED)?.();

      expect(loadWindowGeometry(window.localStorage, "serial-analyzer:invalid-resize-immediate-destroy", {
        minWidth: 640,
        minHeight: 400,
      })).toEqual({ x: 520, y: 280, width: 960, height: 640 });
    }

    // Case G: If WINDOW_MOVED fails to register while WINDOW_RESIZED succeeds,
    // tracking must disarm (cleanup registered listeners) rather than running half-tracked.
    {
      const unlistenDestroyed = vi.fn().mockResolvedValue(undefined);
      const unlistenResized = vi.fn().mockResolvedValue(undefined);
      const child = {
        scaleFactor: vi.fn().mockResolvedValue(1),
        setSize: vi.fn().mockResolvedValue(undefined),
        setPosition: vi.fn().mockResolvedValue(undefined),
        show: vi.fn().mockResolvedValue(undefined),
        outerPosition: vi.fn().mockResolvedValue({ x: 520, y: 280 }),
        innerSize: vi.fn().mockResolvedValue({ width: 960, height: 640 }),
        listen: vi.fn((event: string) => {
          if (event === TauriEvent.WINDOW_DESTROYED) return Promise.resolve(unlistenDestroyed);
          if (event === TauriEvent.WINDOW_MOVED) return Promise.reject(new Error("move listener failed"));
          if (event === TauriEvent.WINDOW_RESIZED) return Promise.resolve(unlistenResized);
          return Promise.resolve(() => {});
        }),
      } as unknown as WebviewWindow;

      await placeAndTrackChildWindow(child, {
        storageKey: "serial-analyzer:partial-registration",
        width: 960,
        height: 640,
        minWidth: 640,
        minHeight: 400,
      });
      await flushMicrotasks();

      expect(child.show).toHaveBeenCalledTimes(1);
      expect(child.outerPosition).not.toHaveBeenCalled();
      expect(unlistenDestroyed).toHaveBeenCalledTimes(1);
      expect(unlistenResized).toHaveBeenCalledTimes(1);
    }
  });

  it("does not save incomplete bounds or late replies resolved after destruction", async () => {
    const { setItem } = stubBrowserWindow();

    const pendingPos = deferred<{ x: number; y: number }>();
    const pendingSize = deferred<{ width: number; height: number }>();
    const listeners = new Map<string, (event?: { payload?: unknown }) => void>();

    const child = {
      scaleFactor: vi.fn().mockResolvedValue(1),
      setSize: vi.fn().mockRejectedValue(new Error("size failed")),
      setPosition: vi.fn().mockResolvedValue(undefined),
      show: vi.fn().mockResolvedValue(undefined),
      outerPosition: vi.fn(() => pendingPos.promise),
      innerSize: vi.fn(() => pendingSize.promise),
      listen: vi.fn(async (event: string, listener: (e?: { payload?: unknown }) => void) => {
        listeners.set(event, listener);
        return () => {};
      }),
    } as unknown as WebviewWindow;

    await placeAndTrackChildWindow(child, {
      storageKey: "serial-analyzer:partial",
      width: 1_000,
      height: 700,
      minWidth: 720,
      minHeight: 480,
    });
    await vi.waitFor(() => expect(child.outerPosition).toHaveBeenCalledTimes(1));

    const moved = listeners.get(TauriEvent.WINDOW_MOVED);
    const destroyed = listeners.get(TauriEvent.WINDOW_DESTROYED);
    expect(moved).toBeDefined();
    expect(destroyed).toBeDefined();

    moved!({ payload: { x: 500, y: 300 } });
    destroyed!();
    expect(setItem).not.toHaveBeenCalled();

    pendingPos.resolve({ x: 500, y: 300 });
    pendingSize.resolve({ width: 1_000, height: 700 });
    await flushMicrotasks();
    expect(setItem).not.toHaveBeenCalled();
    expect(loadWindowGeometry(window.localStorage, "serial-analyzer:partial", {
      minWidth: 720,
      minHeight: 480,
    })).toBeNull();
  });

  it("rechecks ownership after asynchronous placement and before showing a child", async () => {
    vi.stubGlobal("window", {
      get localStorage() {
        throw new DOMException("storage unavailable", "SecurityError");
      },
    });
    const monitors = deferred<never[]>();
    tauriWindow.availableMonitors.mockReturnValue(monitors.promise);
    tauriWindow.getCurrentWindow.mockReturnValue({
      outerPosition: vi.fn().mockResolvedValue({ x: 100, y: 80 }),
      scaleFactor: vi.fn().mockResolvedValue(1),
    });
    const child = {
      scaleFactor: vi.fn().mockResolvedValue(1),
      setSize: vi.fn().mockResolvedValue(undefined),
      setPosition: vi.fn().mockResolvedValue(undefined),
      show: vi.fn().mockResolvedValue(undefined),
    } as unknown as WebviewWindow;
    let current = true;
    const placement = placeAndTrackChildWindow(child, {
      storageKey: null,
      width: 960,
      height: 680,
      minWidth: 640,
      minHeight: 400,
      beforeShow: () => {
        if (!current) throw new Error("stale child launch");
      },
    });

    current = false;
    monitors.resolve([]);

    await expect(placement).rejects.toThrow("stale child launch");
    expect(child.show).not.toHaveBeenCalled();
  });

  it("reports a failed launch when the child is destroyed before show completes", async () => {
    const { setItem } = stubBrowserWindow();
    const pendingShow = deferred<void>();
    const listeners = new Map<string, () => void>();
    const unlisten = vi.fn();
    const child = {
      scaleFactor: vi.fn().mockResolvedValue(1),
      setSize: vi.fn().mockResolvedValue(undefined),
      setPosition: vi.fn().mockResolvedValue(undefined),
      show: vi.fn(() => pendingShow.promise),
      outerPosition: vi.fn(),
      innerSize: vi.fn(),
      listen: vi.fn(async (event: string, listener: () => void) => {
        listeners.set(event, listener);
        return unlisten;
      }),
    } as unknown as WebviewWindow;

    const placement = placeAndTrackChildWindow(child, {
      storageKey: "serial-analyzer:destroy-during-show",
      width: 1_000,
      height: 700,
      minWidth: 720,
      minHeight: 480,
    });
    await flushMicrotasks();
    expect(child.show).toHaveBeenCalledOnce();
    listeners.get(TauriEvent.WINDOW_DESTROYED)!();
    pendingShow.resolve();
    await expect(placement).rejects.toThrow(t("child-window-closed-before-ready"));
    expect(unlisten).toHaveBeenCalledTimes(4);
    expect(child.outerPosition).not.toHaveBeenCalled();
    expect(setItem).not.toHaveBeenCalled();
  });
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}
