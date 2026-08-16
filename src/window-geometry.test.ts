import { afterEach, describe, expect, it, vi } from "vitest";
import type { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { loadWindowGeometry } from "./window-geometry-state";

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

import { placeAndTrackChildWindow } from "./window-geometry";

describe("child window geometry", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.clearAllMocks();
  });

  it("persists the final physical geometry before a child window is destroyed", async () => {
    const values = new Map<string, string>();
    vi.stubGlobal("window", {
      localStorage: {
        getItem(key: string) {
          return values.get(key) ?? null;
        },
        setItem(key: string, value: string) {
          values.set(key, value);
        },
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
      scaleFactor: vi.fn().mockResolvedValue(2),
    });

    let position = { x: 420, y: 360 };
    let size = { width: 1_400, height: 900 };
    let closeListener: (() => void | Promise<void>) | undefined;
    const child = {
      scaleFactor: vi.fn().mockResolvedValue(2),
      setSize: vi.fn().mockResolvedValue(undefined),
      setPosition: vi.fn().mockResolvedValue(undefined),
      show: vi.fn().mockResolvedValue(undefined),
      outerPosition: vi.fn(() => Promise.resolve(position)),
      innerSize: vi.fn(() => Promise.resolve(size)),
      onMoved: vi.fn().mockResolvedValue(() => {}),
      onResized: vi.fn().mockResolvedValue(() => {}),
      onCloseRequested: vi.fn(async (listener: () => void | Promise<void>) => {
        closeListener = listener;
        return () => {};
      }),
      once: vi.fn().mockResolvedValue(() => {}),
    } as unknown as WebviewWindow;

    await placeAndTrackChildWindow(child, {
      storageKey: "detached-pane:view-a",
      width: 960,
      height: 680,
      minWidth: 640,
      minHeight: 400,
    });
    await vi.waitFor(() => expect(child.outerPosition).toHaveBeenCalled());

    position = { x: 1_024, y: 768 };
    size = { width: 1_600, height: 1_000 };
    await closeListener?.();

    expect(loadWindowGeometry(window.localStorage, "detached-pane:view-a", {
      minWidth: 1_280,
      minHeight: 800,
    })).toEqual({ x: 1_024, y: 768, width: 1_600, height: 1_000 });
  });

  it("does not let an older capture overwrite the final close geometry", async () => {
    const values = new Map<string, string>();
    vi.stubGlobal("window", {
      localStorage: {
        getItem(key: string) {
          return values.get(key) ?? null;
        },
        setItem(key: string, value: string) {
          values.set(key, value);
        },
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
      scaleFactor: vi.fn().mockResolvedValue(2),
    });

    const initialPosition = deferred<{ x: number; y: number }>();
    const initialSize = deferred<{ width: number; height: number }>();
    let closeListener: (() => void | Promise<void>) | undefined;
    const child = {
      scaleFactor: vi.fn().mockResolvedValue(2),
      setSize: vi.fn().mockResolvedValue(undefined),
      setPosition: vi.fn().mockResolvedValue(undefined),
      show: vi.fn().mockResolvedValue(undefined),
      outerPosition: vi.fn()
        .mockImplementationOnce(() => initialPosition.promise)
        .mockResolvedValue({ x: 1_024, y: 768 }),
      innerSize: vi.fn()
        .mockImplementationOnce(() => initialSize.promise)
        .mockResolvedValue({ width: 1_600, height: 1_000 }),
      onMoved: vi.fn().mockResolvedValue(() => {}),
      onResized: vi.fn().mockResolvedValue(() => {}),
      onCloseRequested: vi.fn(async (listener: () => void | Promise<void>) => {
        closeListener = listener;
        return () => {};
      }),
      once: vi.fn().mockResolvedValue(() => {}),
    } as unknown as WebviewWindow;

    await placeAndTrackChildWindow(child, {
      storageKey: "detached-pane:view-race",
      width: 960,
      height: 680,
      minWidth: 640,
      minHeight: 400,
    });
    await vi.waitFor(() => expect(child.outerPosition).toHaveBeenCalledTimes(1));

    await closeListener?.();
    initialPosition.resolve({ x: 420, y: 360 });
    initialSize.resolve({ width: 1_400, height: 900 });
    await Promise.resolve();
    await Promise.resolve();

    expect(loadWindowGeometry(window.localStorage, "detached-pane:view-race", {
      minWidth: 1_280,
      minHeight: 800,
    })).toEqual({ x: 1_024, y: 768, width: 1_600, height: 1_000 });
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
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}
