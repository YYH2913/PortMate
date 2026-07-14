export type ScreenLockReason = "manual" | "idle" | "startup" | "restored";

export type ScreenLockMarker = {
  version: 1;
  reason: Exclude<ScreenLockReason, "restored">;
  lockedAt: number;
};

export type DecodedScreenLockMarker = {
  marker: ScreenLockMarker;
  recovered: boolean;
};

export type ScreenLockKeyEvent = {
  code: string;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
  repeat?: boolean;
};

export const SCREEN_LOCK_STORAGE_KEY = "portmate.screenLock.v1";
export const DEFAULT_SCREEN_LOCK_TIMEOUT_MINUTES = 30;
export const MIN_SCREEN_LOCK_TIMEOUT_MINUTES = 1;
export const MAX_SCREEN_LOCK_TIMEOUT_MINUTES = 24 * 60;

export function normalizeScreenLockTimeoutMinutes(value: unknown): number {
  const numeric = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(numeric)) return DEFAULT_SCREEN_LOCK_TIMEOUT_MINUTES;
  return Math.min(MAX_SCREEN_LOCK_TIMEOUT_MINUTES, Math.max(MIN_SCREEN_LOCK_TIMEOUT_MINUTES, Math.trunc(numeric)));
}

export function shouldAutoLockScreen(
  enabled: boolean,
  lastActivityAt: number,
  now: number,
  timeoutMinutes: unknown,
): boolean {
  if (!enabled || !Number.isFinite(lastActivityAt) || !Number.isFinite(now) || now < lastActivityAt) return false;
  return now - lastActivityAt >= normalizeScreenLockTimeoutMinutes(timeoutMinutes) * 60_000;
}

export function isScreenLockShortcut(event: ScreenLockKeyEvent): boolean {
  if (event.repeat || event.code !== "KeyL" || !event.altKey || event.shiftKey) return false;
  return event.ctrlKey !== event.metaKey;
}

export function createScreenLockMarker(reason: Exclude<ScreenLockReason, "restored">, lockedAt = Date.now()): ScreenLockMarker {
  return {
    version: 1,
    reason,
    lockedAt: Number.isFinite(lockedAt) && lockedAt > 0 ? lockedAt : Date.now(),
  };
}

export function parseScreenLockMarker(value: unknown): ScreenLockMarker | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const source = value as Record<string, unknown>;
  if (source.version !== 1 || (source.reason !== "manual" && source.reason !== "idle" && source.reason !== "startup")) return null;
  if (typeof source.lockedAt !== "number" || !Number.isFinite(source.lockedAt) || source.lockedAt <= 0) return null;
  return { version: 1, reason: source.reason, lockedAt: source.lockedAt };
}

export function decodeStoredScreenLockMarker(
  raw: string | null,
  fallbackLockedAt = Date.now(),
): DecodedScreenLockMarker | null {
  if (raw === null) return null;
  try {
    const marker = parseScreenLockMarker(JSON.parse(raw));
    if (marker) return { marker, recovered: false };
  } catch {
    // A present but unreadable marker must not expose a previously locked workspace.
  }
  return {
    marker: createScreenLockMarker("manual", fallbackLockedAt),
    recovered: true,
  };
}
