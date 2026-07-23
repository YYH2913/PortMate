import { describe, expect, it } from "vitest";
import {
  loadWindowGeometry,
  MAX_WINDOW_GEOMETRY_ENTRIES,
  normalizeWindowGeometry,
  saveWindowGeometry,
  windowGeometryOverlapsWorkAreas,
  windowGeometryStorageKey,
  WINDOW_GEOMETRY_STORAGE_KEY,
} from "./window-geometry-state";
import type { WindowGeometryStorage } from "./window-geometry-state";

const constraints = { minWidth: 320, minHeight: 240, maxWidth: 4_000, maxHeight: 3_000 };

describe("window geometry state", () => {
  it("accepts only bounded integer geometry and safe storage keys", () => {
    expect(normalizeWindowGeometry({ x: -120, y: 42, width: 960, height: 680 }, constraints)).toEqual({
      x: -120,
      y: 42,
      width: 960,
      height: 680,
    });
    expect(normalizeWindowGeometry({ x: 0, y: 0, width: 319, height: 680 }, constraints)).toBeNull();
    expect(normalizeWindowGeometry({ x: 0.5, y: 0, width: 960, height: 680 }, constraints)).toBeNull();
    expect(normalizeWindowGeometry({ x: 0, y: 0, width: 960, height: 3_001 }, constraints)).toBeNull();
    expect(normalizeWindowGeometry({ x: 0, y: 0, width: 960, height: 680 }, {
      minWidth: 1_280,
      minHeight: 800,
    })).toBeNull();
    expect(windowGeometryStorageKey("detached-pane", "view-1")).toBe("detached-pane:view-1");
    expect(windowGeometryStorageKey("detached-pane", "view\n1")).toBeNull();
    expect(windowGeometryStorageKey("Detached Pane", "view-1")).toBeNull();
  });

  it("keeps the newest bounded geometry entry for every window", () => {
    const storage = memoryStorage();
    expect(saveWindowGeometry(storage, "detached-pane:view-a", { x: 1, y: 2, width: 960, height: 680 }, constraints)).toBe(true);
    expect(saveWindowGeometry(storage, "detached-pane:view-a", { x: 3, y: 4, width: 1_000, height: 720 }, constraints)).toBe(true);
    expect(loadWindowGeometry(storage, "detached-pane:view-a", constraints)).toEqual({ x: 3, y: 4, width: 1_000, height: 720 });

    for (let index = 0; index <= MAX_WINDOW_GEOMETRY_ENTRIES; index += 1) {
      saveWindowGeometry(storage, `detached-pane:view-${index}`, { x: index, y: 0, width: 960, height: 680 }, constraints);
    }
    expect(loadWindowGeometry(storage, "detached-pane:view-a", constraints)).toBeNull();
    expect(loadWindowGeometry(storage, `detached-pane:view-${MAX_WINDOW_GEOMETRY_ENTRIES}`, constraints)).toEqual({
      x: MAX_WINDOW_GEOMETRY_ENTRIES,
      y: 0,
      width: 960,
      height: 680,
    });
    expect(JSON.parse(storage.getItem(WINDOW_GEOMETRY_STORAGE_KEY) ?? "null").entries).toHaveLength(MAX_WINDOW_GEOMETRY_ENTRIES);
  });

  it("rejects malformed stores and restores only a visible window area", () => {
    const storage = memoryStorage({
      [WINDOW_GEOMETRY_STORAGE_KEY]: "{not-json",
    });
    expect(loadWindowGeometry(storage, "detached-pane:view-a", constraints)).toBeNull();
    expect(windowGeometryOverlapsWorkAreas(
      { x: -1_440, y: 120, width: 960, height: 680 },
      [{ x: -1_920, y: 0, width: 1_920, height: 1_080 }, { x: 0, y: 0, width: 1_920, height: 1_080 }],
    )).toBe(true);
    expect(windowGeometryOverlapsWorkAreas(
      { x: 4_000, y: 120, width: 960, height: 680 },
      [{ x: 0, y: 0, width: 1_920, height: 1_080 }],
    )).toBe(false);
    expect(windowGeometryOverlapsWorkAreas(
      { x: 1_850, y: 1_000, width: 960, height: 680 },
      [{ x: 0, y: 0, width: 1_920, height: 1_080 }],
    )).toBe(false);
  });
});

function memoryStorage(initial: Record<string, string> = {}): WindowGeometryStorage {
  const values = new Map(Object.entries(initial));
  return {
    getItem(key) {
      return values.get(key) ?? null;
    },
    setItem(key, value) {
      values.set(key, value);
    },
  };
}
