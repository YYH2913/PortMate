import { describe, expect, it } from "vitest";
import { proxyDefaults } from "../../src/proxy-settings";
import {
  normalizeSshConnectionSettings,
  SSH_AUTH_ORDER_OPTIONS,
  sshConnectionBounds,
  sshConnectionDefaults,
  sshUsesNoneAuthenticationOnly,
} from "../../src/ssh-connection-settings";
import type { SshConnection } from "../../src/types";

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
  it("identifies only the explicit none-only policy as credential-free", () => {
    const ssh = baseConnection();
    ssh.identityPolicy.authOrder = ["none"];
    expect(sshUsesNoneAuthenticationOnly(ssh)).toBe(true);
    ssh.identityPolicy.authOrder = ["password", "none"];
    expect(sshUsesNoneAuthenticationOnly(ssh)).toBe(false);
    ssh.identityPolicy.authOrder = [];
    expect(sshUsesNoneAuthenticationOnly(ssh)).toBe(false);
  });
  it("offers the supported authentication methods, including explicit none authentication", () => {
    expect(SSH_AUTH_ORDER_OPTIONS).toHaveLength(16);
    expect(new Set(SSH_AUTH_ORDER_OPTIONS).size).toBe(16);
    expect(SSH_AUTH_ORDER_OPTIONS).toContain("keyboard-interactive>public-key");
    expect(SSH_AUTH_ORDER_OPTIONS).toContain("password>keyboard-interactive>public-key");
    expect(SSH_AUTH_ORDER_OPTIONS).toContain("keyboard-interactive");
    expect(SSH_AUTH_ORDER_OPTIONS).toContain("none");
  });

  it("fills health defaults for legacy profiles", () => {
    const legacy = baseConnection() as Partial<SshConnection>;
    delete legacy.reconnectDelayMs;
    delete legacy.reconnectIgnoreHostKeyChanges;
    delete legacy.keepaliveEnabled;
    delete legacy.keepaliveIntervalSeconds;
    delete legacy.keepaliveMaxMissed;
    delete legacy.tcpKeepaliveEnabled;

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

  it("requires an explicit boolean opt-in for ignoring reconnect host-key changes", () => {
    for (const value of [undefined, null, 1, "true", "false", false]) {
      const connection = { ...baseConnection(), reconnectIgnoreHostKeyChanges: value } as unknown as SshConnection;
      expect(normalizeSshConnectionSettings(connection).reconnectIgnoreHostKeyChanges).toBe(false);
    }
    const connection = { ...baseConnection(), reconnectIgnoreHostKeyChanges: true };
    expect(normalizeSshConnectionSettings(connection).reconnectIgnoreHostKeyChanges).toBe(true);
  });

  it("preserves disabled keepalive and valid custom values", () => {
    const normalized = normalizeSshConnectionSettings({
      ...baseConnection(),
      reconnect: false,
      reconnectDelayMs: 2_500,
      keepaliveEnabled: false,
      keepaliveIntervalSeconds: 75,
      keepaliveMaxMissed: 7,
      tcpKeepaliveEnabled: false,
    });

    expect(normalized).toMatchObject({
      reconnect: false,
      reconnectDelayMs: 2_500,
      keepaliveEnabled: false,
      keepaliveIntervalSeconds: 75,
      keepaliveMaxMissed: 7,
      tcpKeepaliveEnabled: false,
    });
  });

  it("allows zero missed replies to keep the session alive without a health timeout", () => {
    const normalized = normalizeSshConnectionSettings({
      ...baseConnection(),
      keepaliveMaxMissed: 0,
    });

    expect(normalized.keepaliveMaxMissed).toBe(0);
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
