import { availableMonitors, getCurrentWindow, PhysicalPosition, PhysicalSize } from "@tauri-apps/api/window";
import type { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import {
  loadWindowGeometry,
  saveWindowGeometry,
  windowGeometryOverlapsWorkAreas,
  windowGeometryStorageKey,
} from "./window-geometry-state";
import type { WindowGeometry, WindowGeometryConstraints, WindowGeometryStorage } from "./window-geometry-state";

const WINDOW_GEOMETRY_SAVE_DELAY_MS = 160;
const WINDOW_CASCADE_STEP_LOGICAL_PIXELS = 48;
const WINDOW_CASCADE_STEPS = 6;

export interface ChildWindowGeometryOptions {
  storageKey: string | null;
  width: number;
  height: number;
  minWidth: number;
  minHeight: number;
  beforeShow?: () => void;
}

let childWindowCascadeIndex = 0;

export function detachedPaneWindowGeometryKey(viewId: string): string | null {
  return windowGeometryStorageKey("detached-pane", viewId);
}

export function serialAnalyzerWindowGeometryKey(sessionId: string): string | null {
  return windowGeometryStorageKey("serial-analyzer", sessionId);
}

export async function placeAndTrackChildWindow(
  child: WebviewWindow,
  options: ChildWindowGeometryOptions,
): Promise<void> {
  const storage = browserStorage();
  const constraints = await childWindowGeometryConstraints(child, options);
  const geometry = storage && options.storageKey
    ? await preferredWindowGeometry(storage, options.storageKey, options, constraints)
    : await cascadeWindowGeometry(options);
  if (geometry) {
    try {
      await child.setSize(new PhysicalSize(geometry.width, geometry.height));
    } catch {
      // The native window keeps its configured default size when restoration is unavailable.
    }
    try {
      await child.setPosition(new PhysicalPosition(geometry.x, geometry.y));
    } catch {
      // The native window remains centered if the saved display is no longer available.
    }
  }
  options.beforeShow?.();
  await child.show();
  if (!storage || !options.storageKey) return;
  const dispose = await trackChildWindowGeometry(child, storage, options.storageKey, constraints);
  void child.once("tauri://destroyed", dispose).catch(() => {});
}

async function preferredWindowGeometry(
  storage: WindowGeometryStorage,
  key: string,
  options: ChildWindowGeometryOptions,
  constraints: WindowGeometryConstraints,
): Promise<WindowGeometry | null> {
  const stored = loadWindowGeometry(storage, key, constraints);
  if (stored && await windowGeometryFitsAvailableDisplays(stored)) return stored;
  return cascadeWindowGeometry(options);
}

async function childWindowGeometryConstraints(
  child: WebviewWindow,
  options: ChildWindowGeometryOptions,
): Promise<WindowGeometryConstraints> {
  let scale = 1;
  try {
    const scaleFactor = await child.scaleFactor();
    if (Number.isFinite(scaleFactor) && scaleFactor > 0) scale = scaleFactor;
  } catch {
    // A failed scale-factor lookup falls back to logical pixels for a usable new window.
  }
  return {
    minWidth: Math.max(1, Math.round(options.minWidth * scale)),
    minHeight: Math.max(1, Math.round(options.minHeight * scale)),
  };
}

async function cascadeWindowGeometry(options: ChildWindowGeometryOptions): Promise<WindowGeometry | null> {
  try {
    const parent = getCurrentWindow();
    const [position, scaleFactor] = await Promise.all([parent.outerPosition(), parent.scaleFactor()]);
    const scale = Number.isFinite(scaleFactor) && scaleFactor > 0 ? scaleFactor : 1;
    const step = ((childWindowCascadeIndex % WINDOW_CASCADE_STEPS) + 1) * WINDOW_CASCADE_STEP_LOGICAL_PIXELS * scale;
    childWindowCascadeIndex += 1;
    const geometry: WindowGeometry = {
      x: Math.round(position.x + step),
      y: Math.round(position.y + step),
      width: Math.round(options.width * scale),
      height: Math.round(options.height * scale),
    };
    return await windowGeometryFitsAvailableDisplays(geometry) ? geometry : null;
  } catch {
    return null;
  }
}

async function windowGeometryFitsAvailableDisplays(geometry: WindowGeometry): Promise<boolean> {
  try {
    const monitors = await availableMonitors();
    if (!monitors.length) return false;
    return windowGeometryOverlapsWorkAreas(geometry, monitors.map((monitor) => ({
      x: monitor.workArea.position.x,
      y: monitor.workArea.position.y,
      width: monitor.workArea.size.width,
      height: monitor.workArea.size.height,
    })));
  } catch {
    return false;
  }
}

async function trackChildWindowGeometry(
  child: WebviewWindow,
  storage: WindowGeometryStorage,
  key: string,
  constraints: WindowGeometryConstraints,
): Promise<() => void> {
  let disposed = false;
  let timeout: number | null = null;
  let captureGeneration = 0;
  const capture = async () => {
    if (disposed) return;
    const generation = ++captureGeneration;
    try {
      const [position, size] = await Promise.all([child.outerPosition(), child.innerSize()]);
      if (disposed || generation !== captureGeneration) return;
      saveWindowGeometry(storage, key, {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
      }, constraints);
    } catch {
      // Geometry persistence is best-effort and must never affect a usable child window.
    }
  };
  const scheduleCapture = () => {
    if (disposed) return;
    if (timeout !== null) window.clearTimeout(timeout);
    timeout = window.setTimeout(() => {
      timeout = null;
      void capture();
    }, WINDOW_GEOMETRY_SAVE_DELAY_MS);
  };
  const listeners = await Promise.allSettled([
    child.onMoved(scheduleCapture),
    child.onResized(scheduleCapture),
    child.onCloseRequested(async () => {
      if (timeout !== null) {
        window.clearTimeout(timeout);
        timeout = null;
      }
      await capture();
    }),
  ]);
  void capture();
  return () => {
    disposed = true;
    captureGeneration += 1;
    if (timeout !== null) window.clearTimeout(timeout);
    for (const listener of listeners) {
      if (listener.status === "fulfilled") listener.value();
    }
  };
}

function browserStorage(): WindowGeometryStorage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}
