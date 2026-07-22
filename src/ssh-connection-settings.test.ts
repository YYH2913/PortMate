import { describe, expect, it } from "vitest";
import { proxyDefaults } from "./proxy-settings";
import {
  normalizeSshConnectionSettings,
  SSH_AUTH_ORDER_OPTIONS,
  sshConnectionBounds,
  sshConnectionDefaults,
} from "./ssh-connection-settings";
import type { SshConnection } from "./types";

function baseConnection(): SshConnection {
  return {
    endpoint: { host: "device.example", port: 22 },
    username: "root",
    reconnect: true,
    ...sshConnectionDefaults,
    proxy: proxyDefaults,
    passwordSecretRef: null,
    passphraseSecretRef: null,
    hostKeyPolicy: { mode: "strict", alias: "device", trustScope: "profile", allowRotation: false, checkIp: false },
    trustedHostKeys: [],
    identityPolicy: { identitiesOnly: true, authOrder: ["public-key"], recordSuccess: true, lastSuccessful: null },
    identityRefs: [],
    agentPolicy: { enabled: false, forwarding: false, offerMode: "disabled" },
    jumps: [],
    tunnels: [],
  };
}

describe("SSH connection settings", () => {
  it("offers every ordered non-empty subset of the supported authentication methods", () => {
    expect(SSH_AUTH_ORDER_OPTIONS).toHaveLength(15);
    expect(new Set(SSH_AUTH_ORDER_OPTIONS).size).toBe(15);
    expect(SSH_AUTH_ORDER_OPTIONS).toContain("keyboard-interactive>public-key");
    expect(SSH_AUTH_ORDER_OPTIONS).toContain("password>keyboard-interactive>public-key");
    expect(SSH_AUTH_ORDER_OPTIONS).toContain("keyboard-interactive");
  });

  it("fills health defaults for legacy profiles", () => {
    const legacy = baseConnection() as Partial<SshConnection>;
    delete legacy.reconnectDelayMs;
    delete legacy.keepaliveEnabled;
    delete legacy.keepaliveIntervalSeconds;
    delete legacy.keepaliveMaxMissed;

    expect(normalizeSshConnectionSettings(legacy as SshConnection)).toMatchObject(sshConnectionDefaults);
  });

  it("clamps and truncates operational settings", () => {
    const normalized = normalizeSshConnectionSettings({
      ...baseConnection(),
      reconnectDelayMs: -1,
      keepaliveIntervalSeconds: Number.MAX_SAFE_INTEGER,
      keepaliveMaxMissed: 4.9,
    });

    expect(normalized.reconnectDelayMs).toBe(sshConnectionBounds.reconnectDelayMs.min);
    expect(normalized.keepaliveIntervalSeconds).toBe(sshConnectionBounds.keepaliveIntervalSeconds.max);
    expect(normalized.keepaliveMaxMissed).toBe(4);
  });

  it("preserves disabled keepalive and valid custom values", () => {
    const normalized = normalizeSshConnectionSettings({
      ...baseConnection(),
      reconnect: false,
      reconnectDelayMs: 2_500,
      keepaliveEnabled: false,
      keepaliveIntervalSeconds: 75,
      keepaliveMaxMissed: 7,
    });

    expect(normalized).toMatchObject({
      reconnect: false,
      reconnectDelayMs: 2_500,
      keepaliveEnabled: false,
      keepaliveIntervalSeconds: 75,
      keepaliveMaxMissed: 7,
    });
  });

  it("keeps the successful-auth hint inside the current authentication policy", () => {
    const disabledMethod = normalizeSshConnectionSettings({
      ...baseConnection(),
      identityPolicy: {
        identitiesOnly: true,
        authOrder: ["password"],
        recordSuccess: true,
        lastSuccessful: "public-key",
      },
    });
    expect(disabledMethod.identityPolicy).toMatchObject({
      authOrder: ["password"],
      recordSuccess: true,
      lastSuccessful: null,
    });

    const recordingDisabled = normalizeSshConnectionSettings({
      ...baseConnection(),
      identityPolicy: {
        identitiesOnly: true,
        authOrder: ["password", "public-key"],
        recordSuccess: false,
        lastSuccessful: "public-key",
      },
    });
    expect(recordingDisabled.identityPolicy.lastSuccessful).toBeNull();

    const legacyAliases = normalizeSshConnectionSettings({
      ...baseConnection(),
      identityPolicy: {
        identitiesOnly: true,
        authOrder: ["publickey", "password", "publickey"] as unknown as SshConnection["identityPolicy"]["authOrder"],
        recordSuccess: true,
        lastSuccessful: "publickey" as unknown as SshConnection["identityPolicy"]["lastSuccessful"],
      },
    });
    expect(legacyAliases.identityPolicy).toMatchObject({
      authOrder: ["public-key", "password"],
      lastSuccessful: "public-key",
    });
  });
});
