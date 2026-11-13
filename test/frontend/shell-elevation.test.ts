import { describe, expect, it } from "vitest";
import { createShellConnection } from "../../src/session-profile-helpers";
import { localTerminalTag, matchesLocalTerminal, shellElevationConsentMatches, shellElevationRequested } from "../../src/shell-elevation";
import { startupCredentialsReady } from "../../src/startup-connection-state";
import type { SessionProfile } from "../../src/types";

function profile(elevated = false, tag = localTerminalTag(elevated)): SessionProfile {
  return { kind: "shell", name: "Local Shell", tags: [tag], connection: { ...createShellConnection(), elevated } } as SessionProfile;
}

describe("local terminal elevation boundaries", () => {
  it("defaults normal and never silently restores an elevated session, even after vault unlock", () => {
    expect(createShellConnection().elevated).toBe(false);
    for (const unlocked of [true, false]) {
      expect(startupCredentialsReady(profile(true), unlocked)).toBe(false);
      expect(startupCredentialsReady(profile(), unlocked)).toBe(true);
    }
    const old = profile();
    if (old.connection.kind === "shell") delete old.connection.elevated;
    expect(shellElevationRequested(old)).toBe(false);
    expect(startupCredentialsReady(old, true)).toBe(true);
  });

  it("rejects configuration changes between manual consent and save, including normal-to-elevated changes", () => {
    const shown = profile(true);
    if (shown.connection.kind !== "shell") throw Error("fixture");
    const consent = { ...shown.connection, args: [...shown.connection.args] };
    expect(shellElevationConsentMatches(shown, consent)).toBe(true);
    expect(shellElevationConsentMatches(shown, null)).toBe(false);
    expect(shellElevationConsentMatches(profile(), null)).toBe(true);
    for (const change of [
      { program: "other.exe" }, { args: ["--different"] }, { cwd: "/other" }, { elevated: false },
    ]) {
      const changed = { ...shown, connection: { ...shown.connection, ...change } };
      expect(shellElevationConsentMatches(changed, consent)).toBe(false);
    }
  });

  it("keeps normal and elevated local terminals separate regardless of rename or stale tags", () => {
    for (const elevated of [true, false]) {
      const saved = profile(elevated);
      expect(matchesLocalTerminal(saved, elevated)).toBe(true);
      expect(matchesLocalTerminal(saved, !elevated)).toBe(false);
      saved.name = "renamed";
      expect(matchesLocalTerminal(saved, elevated)).toBe(true);
    }
    expect(matchesLocalTerminal(profile(true, localTerminalTag(false)), false)).toBe(false);
    const legacy = { ...profile(), name: "local-terminal", tags: [] };
    expect(matchesLocalTerminal(legacy, false)).toBe(true);
    expect(matchesLocalTerminal(legacy, true)).toBe(false);
  });
});
