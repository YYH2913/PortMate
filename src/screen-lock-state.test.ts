import { describe, expect, it } from "vitest";
import {
  createScreenLockMarker,
  decodeStoredScreenLockMarker,
  DEFAULT_SCREEN_LOCK_TIMEOUT_MINUTES,
  isScreenLockShortcut,
  MAX_SCREEN_LOCK_TIMEOUT_MINUTES,
  normalizeScreenLockTimeoutMinutes,
  parseScreenLockMarker,
  shouldAutoLockScreen,
} from "./screen-lock-state";

describe("screen lock state", () => {
  it("normalizes the WindTerm-style idle timeout within explicit bounds", () => {
    expect(normalizeScreenLockTimeoutMinutes(undefined)).toBe(DEFAULT_SCREEN_LOCK_TIMEOUT_MINUTES);
    expect(normalizeScreenLockTimeoutMinutes("15")).toBe(15);
    expect(normalizeScreenLockTimeoutMinutes(0)).toBe(1);
    expect(normalizeScreenLockTimeoutMinutes(99_999)).toBe(MAX_SCREEN_LOCK_TIMEOUT_MINUTES);
  });

  it("locks only after the configured idle deadline", () => {
    const lastActivityAt = 1_000_000;
    expect(shouldAutoLockScreen(true, lastActivityAt, lastActivityAt + 299_999, 5)).toBe(false);
    expect(shouldAutoLockScreen(true, lastActivityAt, lastActivityAt + 300_000, 5)).toBe(true);
    expect(shouldAutoLockScreen(false, lastActivityAt, lastActivityAt + 300_000, 5)).toBe(false);
    expect(shouldAutoLockScreen(true, lastActivityAt, lastActivityAt - 1, 5)).toBe(false);
  });

  it("recognizes the exact WindTerm desktop and macOS shortcuts", () => {
    const base = { code: "KeyL", altKey: true, shiftKey: false, repeat: false };
    expect(isScreenLockShortcut({ ...base, ctrlKey: true, metaKey: false })).toBe(true);
    expect(isScreenLockShortcut({ ...base, ctrlKey: false, metaKey: true })).toBe(true);
    expect(isScreenLockShortcut({ ...base, ctrlKey: true, metaKey: true })).toBe(false);
    expect(isScreenLockShortcut({ ...base, ctrlKey: true, metaKey: false, shiftKey: true })).toBe(false);
    expect(isScreenLockShortcut({ ...base, ctrlKey: true, metaKey: false, repeat: true })).toBe(false);
  });

  it("round-trips only versioned lock markers", () => {
    const marker = createScreenLockMarker("idle", 42);
    expect(parseScreenLockMarker(marker)).toEqual(marker);
    const startup = createScreenLockMarker("startup", 43);
    expect(parseScreenLockMarker(startup)).toEqual(startup);
    expect(parseScreenLockMarker({ ...marker, version: 2 })).toBeNull();
    expect(parseScreenLockMarker({ ...marker, reason: "restored" })).toBeNull();
    expect(parseScreenLockMarker({ ...marker, lockedAt: Number.NaN })).toBeNull();
  });

  it("fails closed when a persisted lock marker is damaged", () => {
    const marker = createScreenLockMarker("idle", 42);
    expect(decodeStoredScreenLockMarker(null, 99)).toBeNull();
    expect(decodeStoredScreenLockMarker(JSON.stringify(marker), 99)).toEqual({
      marker,
      recovered: false,
    });
    expect(decodeStoredScreenLockMarker("not-json", 99)).toEqual({
      marker: createScreenLockMarker("manual", 99),
      recovered: true,
    });
    expect(decodeStoredScreenLockMarker(JSON.stringify({ ...marker, version: 2 }), 100)).toEqual({
      marker: createScreenLockMarker("manual", 100),
      recovered: true,
    });
  });
});
