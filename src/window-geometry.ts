import { TauriEvent } from "@tauri-apps/api/event";
import { availableMonitors, getCurrentWindow, PhysicalPosition, PhysicalSize } from "@tauri-apps/api/window";
import type { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { t } from "./i18n";
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
  const tracking = storage && options.storageKey
    ? await armChildWindowGeometryTracking(child, storage, options.storageKey, constraints)
    : { status: "unavailable" as const };
  if (tracking.status === "destroyed") throw new Error(t("child-window-closed-before-ready"));
  try {
    options.beforeShow?.();
    await child.show();
  } catch (error) {
    if (tracking.status === "ready") tracking.dispose();
    throw error;
  }
  if (tracking.status === "ready" && !tracking.activate()) {
    throw new Error(t("child-window-closed-before-ready"));
  }
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

function isWindowsMinimizedPosition(x: number, y: number): boolean {
  return x <= -32_000 && y <= -32_000;
}

function isPositiveWindowSize(width: number, height: number): boolean {
  return width > 0 && height > 0;
}

function invokeUnlisten(unlisten: () => void | Promise<void>): void {
  try {
    void Promise.resolve(unlisten()).catch(() => {});
  } catch {
    // Ignore best-effort listener cleanup errors after window teardown.
  }
}

type MinimizedState = "normal" | "minimized" | "unknown";

type ArmedChildGeometryTracking =
  | { status: "ready"; activate: () => boolean; dispose: () => void }
  | { status: "destroyed" }
  | { status: "unavailable" };

async function armChildWindowGeometryTracking(
  child: WebviewWindow,
  storage: WindowGeometryStorage,
  key: string,
  constraints: WindowGeometryConstraints,
): Promise<ArmedChildGeometryTracking> {
  let disposed = false;
  let persistenceEnabled = false;
  let timeout: number | null = null;
  let positionGeneration = 0;
  let sizeGeneration = 0;
  let querySequence = 0;
  let appliedPositionQuery = 0;
  let appliedSizeQuery = 0;
  let positionValid = false;
  let sizeValid = false;
  let latestPosition: { x: number; y: number } | null = null;
  let latestSize: { width: number; height: number } | null = null;
  let minimizeState: MinimizedState = "normal";
  let dirty = false;
  const unlisteners: Array<() => void | Promise<void>> = [];
  let resolveDestroyed!: () => void;
  const destroyedPromise = new Promise<void>((resolve) => {
    resolveDestroyed = resolve;
  });

  const invalidateLiveBounds = (nextMinimizeState: "minimized" | "unknown") => {
    minimizeState = nextMinimizeState;
    positionGeneration += 1;
    sizeGeneration += 1;
    querySequence += 1;
    appliedPositionQuery = querySequence;
    appliedSizeQuery = querySequence;
    latestPosition = null;
    latestSize = null;
    positionValid = false;
    sizeValid = false;
    dirty = false;
  };

  const checkMinimized = async (): Promise<MinimizedState> => {
    if (typeof child.isMinimized !== "function") return "normal";
    try {
      return (await child.isMinimized()) ? "minimized" : "normal";
    } catch {
      return "unknown";
    }
  };

  const persistKnownGeometry = (): boolean => {
    if (
      !persistenceEnabled
      || minimizeState !== "normal"
      || !positionValid
      || !sizeValid
      || !latestPosition
      || !latestSize
    ) {
      return false;
    }
    if (
      !isPositiveWindowSize(latestSize.width, latestSize.height)
      || isWindowsMinimizedPosition(latestPosition.x, latestPosition.y)
    ) {
      return false;
    }
    const next: WindowGeometry = {
      x: latestPosition.x,
      y: latestPosition.y,
      width: latestSize.width,
      height: latestSize.height,
    };
    dirty = false;
    saveWindowGeometry(storage, key, next, constraints);
    return true;
  };

  const capture = async () => {
    if (disposed || !persistenceEnabled) return;
    const myQuerySequence = ++querySequence;
    const capturedPositionGeneration = positionGeneration;
    const capturedSizeGeneration = sizeGeneration;
    try {
      const beforeState = await checkMinimized();
      if (disposed) return;
      if (beforeState !== "normal") {
        if (
          myQuerySequence === querySequence
          && capturedPositionGeneration === positionGeneration
          && capturedSizeGeneration === sizeGeneration
        ) {
          invalidateLiveBounds(beforeState);
        }
        return;
      }

      const [positionResult, sizeResult] = await Promise.allSettled([
        child.outerPosition(),
        child.innerSize(),
      ]);
      if (disposed) return;

      const afterState = await checkMinimized();
      if (disposed) return;
      if (afterState !== "normal") {
        if (
          myQuerySequence === querySequence
          && capturedPositionGeneration === positionGeneration
          && capturedSizeGeneration === sizeGeneration
        ) {
          invalidateLiveBounds(afterState);
        }
        return;
      }

      const generationsMatch =
        capturedPositionGeneration === positionGeneration
        && capturedSizeGeneration === sizeGeneration;
      if (!generationsMatch && minimizeState !== "normal") {
        // Do not accept a partial capture while exiting minimized/unknown state if newer
        // untrusted events arrived mid-capture; wait for the rescheduled capture instead.
        return;
      }
      if (generationsMatch && myQuerySequence > Math.max(appliedPositionQuery, appliedSizeQuery)) {
        minimizeState = "normal";
      }

      const position = positionResult.status === "fulfilled" ? positionResult.value : null;
      const size = sizeResult.status === "fulfilled" ? sizeResult.value : null;
      if (
        (size && !isPositiveWindowSize(size.width, size.height))
        || (position && isWindowsMinimizedPosition(position.x, position.y))
      ) {
        return;
      }

      let updated = false;
      if (
        position
        && Number.isFinite(position.x)
        && Number.isFinite(position.y)
        && capturedPositionGeneration === positionGeneration
        && myQuerySequence > appliedPositionQuery
      ) {
        appliedPositionQuery = myQuerySequence;
        latestPosition = { x: position.x, y: position.y };
        positionValid = true;
        dirty = true;
        updated = true;
      }
      if (
        size
        && Number.isFinite(size.width)
        && Number.isFinite(size.height)
        && capturedSizeGeneration === sizeGeneration
        && myQuerySequence > appliedSizeQuery
      ) {
        appliedSizeQuery = myQuerySequence;
        latestSize = { width: size.width, height: size.height };
        sizeValid = true;
        dirty = true;
        updated = true;
      }
      if (updated && dirty && generationsMatch) {
        minimizeState = "normal";
        persistKnownGeometry();
      }
    } catch {
      // Geometry persistence is best-effort and must never affect a usable child window.
    }
  };

  const scheduleCapture = () => {
    if (disposed || !persistenceEnabled) return;
    if (timeout !== null) window.clearTimeout(timeout);
    timeout = window.setTimeout(() => {
      timeout = null;
      void capture();
    }, WINDOW_GEOMETRY_SAVE_DELAY_MS);
  };

  const handleMoved = (event?: { payload?: unknown }) => {
    if (disposed) return;
    if (minimizeState !== "normal") {
      // Do not trust or cache live payloads emitted while minimized/unknown;
      // defer to a verified full capture() once the window settles.
      positionGeneration += 1;
      sizeGeneration += 1;
      latestPosition = null;
      latestSize = null;
      positionValid = false;
      sizeValid = false;
      dirty = false;
      scheduleCapture();
      return;
    }
    const payload = event?.payload as { x?: unknown; y?: unknown } | null | undefined;
    if (
      typeof payload?.x === "number"
      && typeof payload?.y === "number"
      && Number.isFinite(payload.x)
      && Number.isFinite(payload.y)
    ) {
      if (isWindowsMinimizedPosition(payload.x, payload.y)) return;
      positionGeneration += 1;
      latestPosition = { x: payload.x, y: payload.y };
      positionValid = true;
      dirty = true;
      scheduleCapture();
    } else {
      positionGeneration += 1;
      positionValid = false;
      scheduleCapture();
    }
  };

  const handleResized = (event?: { payload?: unknown }) => {
    if (disposed) return;
    if (minimizeState !== "normal") {
      positionGeneration += 1;
      sizeGeneration += 1;
      latestPosition = null;
      latestSize = null;
      positionValid = false;
      sizeValid = false;
      dirty = false;
      scheduleCapture();
      return;
    }
    const payload = event?.payload as { width?: unknown; height?: unknown } | null | undefined;
    if (
      typeof payload?.width === "number"
      && typeof payload?.height === "number"
      && Number.isFinite(payload.width)
      && Number.isFinite(payload.height)
    ) {
      if (!isPositiveWindowSize(payload.width, payload.height)) return;
      sizeGeneration += 1;
      latestSize = { width: payload.width, height: payload.height };
      sizeValid = true;
      dirty = true;
      scheduleCapture();
    } else {
      sizeGeneration += 1;
      sizeValid = false;
      scheduleCapture();
    }
  };

  const handleScaleChanged = () => {
    if (disposed) return;
    positionGeneration += 1;
    sizeGeneration += 1;
    positionValid = false;
    sizeValid = false;
    scheduleCapture();
  };

  const dispose = (flush: boolean) => {
    if (disposed) return;
    disposed = true;
    positionGeneration += 1;
    sizeGeneration += 1;
    querySequence += 1;
    if (timeout !== null) {
      window.clearTimeout(timeout);
      timeout = null;
    }
    if (flush && persistenceEnabled && dirty && minimizeState === "normal" && positionValid && sizeValid) {
      persistKnownGeometry();
    }
    for (const unlisten of unlisteners.splice(0)) {
      invokeUnlisten(unlisten);
    }
    resolveDestroyed();
  };

  // Arm destruction cleanup BEFORE showing the child window and never attach a
  // close-requested listener from the owner window: in Tauri v2, any JS listener
  // on close-requested causes Rust to call prevent_close() and delegate
  // destruction back to the listener. Capturing live move/resize payloads via raw
  // child.listen() and flushing on WINDOW_DESTROYED preserves final geometry
  // while keeping native OS close and child close() immediate and unblockable.
  if (typeof child.listen !== "function") {
    return { status: "unavailable" };
  }
  let destroyedRegistered = false;
  const destroyedRegPromise = Promise.resolve()
    .then(() => child.listen(TauriEvent.WINDOW_DESTROYED, () => dispose(true)))
    .then(
      (unlistenDestroyed) => {
        destroyedRegistered = true;
        if (disposed) {
          invokeUnlisten(unlistenDestroyed);
          return;
        }
        unlisteners.push(unlistenDestroyed);
      },
      () => {},
    );
  await Promise.race([destroyedRegPromise, destroyedPromise]);
  if (disposed) {
    return { status: "destroyed" };
  }
  if (!destroyedRegistered) {
    return { status: "unavailable" };
  }

  let movedRegistered = false;
  let resizedRegistered = false;
  const register = (
    promise: Promise<() => void | Promise<void>>,
    onRegistered?: () => void,
  ) =>
    promise.then(
      (unlisten) => {
        if (disposed) {
          invokeUnlisten(unlisten);
          return;
        }
        unlisteners.push(unlisten);
        onRegistered?.();
      },
      () => {},
    );

  await Promise.race([
    Promise.all([
      register(child.listen(TauriEvent.WINDOW_MOVED, handleMoved), () => {
        movedRegistered = true;
      }),
      register(child.listen(TauriEvent.WINDOW_RESIZED, handleResized), () => {
        resizedRegistered = true;
      }),
      register(child.listen(TauriEvent.WINDOW_SCALE_FACTOR_CHANGED, handleScaleChanged)),
    ]),
    destroyedPromise,
  ]);

  if (disposed) {
    return { status: "destroyed" };
  }
  if (!movedRegistered || !resizedRegistered) {
    dispose(false);
    return { status: "unavailable" };
  }

  return {
    status: "ready",
    activate: () => {
      if (disposed) return false;
      persistenceEnabled = true;
      void capture();
      return true;
    },
    dispose: () => dispose(false),
  };
}

function browserStorage(): WindowGeometryStorage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}
