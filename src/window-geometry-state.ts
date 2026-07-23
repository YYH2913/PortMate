export const WINDOW_GEOMETRY_STORAGE_KEY = "portmate.windowGeometry.v1";
export const MAX_WINDOW_GEOMETRY_ENTRIES = 32;
export const MAX_WINDOW_GEOMETRY_KEY_CHARACTERS = 384;
export const MAX_WINDOW_GEOMETRY_COORDINATE = 100_000;
export const MAX_WINDOW_GEOMETRY_SIZE = 32_768;
export const MIN_WINDOW_GEOMETRY_VISIBLE_PIXELS = 96;

export interface WindowGeometry {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface WindowGeometryConstraints {
  minWidth: number;
  minHeight: number;
  maxWidth?: number;
  maxHeight?: number;
}

export interface WindowGeometryStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export interface WindowGeometryWorkArea {
  x: number;
  y: number;
  width: number;
  height: number;
}

type WindowGeometryEntry = {
  key: string;
  geometry: WindowGeometry;
};

type WindowGeometryStore = {
  version: 1;
  entries: WindowGeometryEntry[];
};

const storedGeometryConstraints: WindowGeometryConstraints = {
  minWidth: 1,
  minHeight: 1,
};

export function windowGeometryStorageKey(scope: string, id: string): string | null {
  if (!/^[a-z][a-z0-9-]{0,63}$/.test(scope)) return null;
  const normalizedId = normalizeWindowGeometryKeyPart(id);
  if (!normalizedId) return null;
  const key = `${scope}:${normalizedId}`;
  return [...key].length <= MAX_WINDOW_GEOMETRY_KEY_CHARACTERS ? key : null;
}

export function normalizeWindowGeometry(
  value: unknown,
  constraints: WindowGeometryConstraints = storedGeometryConstraints,
): WindowGeometry | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const source = value as Record<string, unknown>;
  const minWidth = normalizedMinimum(constraints.minWidth);
  const minHeight = normalizedMinimum(constraints.minHeight);
  const maxWidth = normalizedMaximum(constraints.maxWidth, minWidth);
  const maxHeight = normalizedMaximum(constraints.maxHeight, minHeight);
  const x = normalizedInteger(source.x, -MAX_WINDOW_GEOMETRY_COORDINATE, MAX_WINDOW_GEOMETRY_COORDINATE);
  const y = normalizedInteger(source.y, -MAX_WINDOW_GEOMETRY_COORDINATE, MAX_WINDOW_GEOMETRY_COORDINATE);
  const width = normalizedInteger(source.width, minWidth, maxWidth);
  const height = normalizedInteger(source.height, minHeight, maxHeight);
  return x === null || y === null || width === null || height === null ? null : { x, y, width, height };
}

export function loadWindowGeometry(
  storage: WindowGeometryStorage,
  key: string,
  constraints: WindowGeometryConstraints,
): WindowGeometry | null {
  const normalizedKey = normalizeWindowGeometryKey(key);
  if (!normalizedKey) return null;
  const store = readWindowGeometryStore(storage);
  const entry = store.entries.find((candidate) => candidate.key === normalizedKey);
  return entry ? normalizeWindowGeometry(entry.geometry, constraints) : null;
}

export function saveWindowGeometry(
  storage: WindowGeometryStorage,
  key: string,
  geometry: WindowGeometry,
  constraints: WindowGeometryConstraints,
): boolean {
  const normalizedKey = normalizeWindowGeometryKey(key);
  const normalizedGeometry = normalizeWindowGeometry(geometry, constraints);
  if (!normalizedKey || !normalizedGeometry) return false;
  const store = readWindowGeometryStore(storage);
  const entries = store.entries.filter((entry) => entry.key !== normalizedKey);
  entries.push({ key: normalizedKey, geometry: normalizedGeometry });
  const next: WindowGeometryStore = {
    version: 1,
    entries: entries.slice(-MAX_WINDOW_GEOMETRY_ENTRIES),
  };
  try {
    storage.setItem(WINDOW_GEOMETRY_STORAGE_KEY, JSON.stringify(next));
    return true;
  } catch {
    return false;
  }
}

export function windowGeometryOverlapsWorkAreas(
  geometry: WindowGeometry,
  workAreas: readonly WindowGeometryWorkArea[],
  minimumVisiblePixels = MIN_WINDOW_GEOMETRY_VISIBLE_PIXELS,
): boolean {
  const normalizedGeometry = normalizeWindowGeometry(geometry, storedGeometryConstraints);
  const minimum = normalizedInteger(minimumVisiblePixels, 1, MAX_WINDOW_GEOMETRY_SIZE);
  if (!normalizedGeometry || minimum === null) return false;
  return workAreas.some((workArea) => {
    const normalizedArea = normalizeWindowGeometry(workArea, storedGeometryConstraints);
    if (!normalizedArea) return false;
    const visibleWidth = Math.min(
      normalizedGeometry.x + normalizedGeometry.width,
      normalizedArea.x + normalizedArea.width,
    ) - Math.max(normalizedGeometry.x, normalizedArea.x);
    const visibleHeight = Math.min(
      normalizedGeometry.y + normalizedGeometry.height,
      normalizedArea.y + normalizedArea.height,
    ) - Math.max(normalizedGeometry.y, normalizedArea.y);
    return visibleWidth >= minimum && visibleHeight >= minimum;
  });
}

function readWindowGeometryStore(storage: WindowGeometryStorage): WindowGeometryStore {
  try {
    return normalizeWindowGeometryStore(JSON.parse(storage.getItem(WINDOW_GEOMETRY_STORAGE_KEY) ?? "null"));
  } catch {
    return { version: 1, entries: [] };
  }
}

function normalizeWindowGeometryStore(value: unknown): WindowGeometryStore {
  if (!value || typeof value !== "object" || Array.isArray(value)) return { version: 1, entries: [] };
  const source = value as Record<string, unknown>;
  if (source.version !== 1 || !Array.isArray(source.entries)) return { version: 1, entries: [] };
  const entries: WindowGeometryEntry[] = [];
  for (const rawEntry of source.entries) {
    if (!rawEntry || typeof rawEntry !== "object" || Array.isArray(rawEntry)) continue;
    const entry = rawEntry as Record<string, unknown>;
    const key = typeof entry.key === "string" ? normalizeWindowGeometryKey(entry.key) : null;
    const geometry = normalizeWindowGeometry(entry.geometry, storedGeometryConstraints);
    if (!key || !geometry) continue;
    const existingIndex = entries.findIndex((candidate) => candidate.key === key);
    if (existingIndex >= 0) entries.splice(existingIndex, 1);
    entries.push({ key, geometry });
  }
  return { version: 1, entries: entries.slice(-MAX_WINDOW_GEOMETRY_ENTRIES) };
}

function normalizeWindowGeometryKey(value: string): string | null {
  const key = value.trim();
  if (!key || /[\u0000-\u001f\u007f]/.test(key) || [...key].length > MAX_WINDOW_GEOMETRY_KEY_CHARACTERS) return null;
  return key;
}

function normalizeWindowGeometryKeyPart(value: string): string | null {
  const key = value.trim();
  if (!key || /[\u0000-\u001f\u007f]/.test(key)) return null;
  return key;
}

function normalizedMinimum(value: number): number {
  return normalizedInteger(value, 1, MAX_WINDOW_GEOMETRY_SIZE) ?? 1;
}

function normalizedMaximum(value: number | undefined, minimum: number): number {
  return normalizedInteger(value, minimum, MAX_WINDOW_GEOMETRY_SIZE) ?? MAX_WINDOW_GEOMETRY_SIZE;
}

function normalizedInteger(value: unknown, minimum: number, maximum: number): number | null {
  if (typeof value !== "number" || !Number.isFinite(value) || !Number.isInteger(value)) return null;
  return value >= minimum && value <= maximum ? value : null;
}
