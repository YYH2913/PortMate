import { describe, expect, it } from "vitest";
import { startupCredentialsReady } from "../../src/startup-connection-state";
import { createSshConnection } from "../../src/session-profile-helpers";
import type { SessionProfile, SshConnection } from "../../src/types";

function createSessionDraft(): SessionProfile {
  return { kind: "ssh", connection: createSshConnection() } as SessionProfile;
}

describe("vault-dependent startup", () => {
  it("does not require unused password secrets for none authentication", () => {
    const profile = createSessionDraft();
    const ssh = profile.connection as SshConnection;
    ssh.identityPolicy.authOrder = ["none"];
    ssh.passwordSecretRef = "stronghold:unused";
    expect(startupCredentialsReady(profile, false)).toBe(true);
    ssh.proxy.passwordSecretRef = "stronghold:needed-proxy";
    expect(startupCredentialsReady(profile, false)).toBe(false);
  });
  it("defers stored passwords until unlock and does not block credential-free sessions", () => {
    const profile = createSessionDraft();
    profile.kind = "ssh";
    const ssh = profile.connection as SshConnection;
    expect(startupCredentialsReady(profile, false)).toBe(true);
    ssh.passwordSecretRef = "stronghold:password";
    expect(startupCredentialsReady(profile, false)).toBe(false);
    expect(startupCredentialsReady(profile, true)).toBe(true);
  });
  it("also defers Stronghold proxy, private-key and jump credentials", () => {
    for (const field of ["proxy", "key", "jump"]) {
      const profile = createSessionDraft();
      profile.kind = "ssh";
      const ssh = profile.connection as SshConnection;
      if (field === "proxy") ssh.proxy.passwordSecretRef = "stronghold:proxy";
      if (field === "key") ssh.identityRefs = [{ id: "k", label: "key", source: "profile-vault", secretRef: "stronghold:key" }];
      if (field === "jump") ssh.jumps = [{ host: "jump", port: 22, username: "u", passwordSecretRef: "stronghold:jump" }];
      expect(startupCredentialsReady(profile, false)).toBe(false);
    }
  });
});
