import { describe, expect, it } from "vitest";
import { createSshConnection } from "./session-profile-helpers";
import { sshHealthProfileSnapshotMatches } from "./ssh-health-profile-state";
import type { SessionProfile, TrustedHostKey } from "./types";

function sshProfile(): SessionProfile {
  const connection = createSshConnection();
  connection.endpoint = { host: "router.example", port: 22 };
  connection.username = "root";
  connection.jumps = [{
    host: "jump.example",
    port: 22,
    username: "ops",
    passwordSecretRef: null,
    passphraseSecretRef: null,
    identityRef: null,
    hostKeyPolicy: null,
  }];
  return {
    id: "ssh-1",
    name: "Router",
    kind: "ssh",
    group: "",
    tags: [],
    connection,
    terminal: {
      term: "xterm-256color",
      rows: 32,
      cols: 120,
      scrollback: 10_000,
      fontFamily: "monospace",
      fontSize: 13,
      theme: "portmate-dark",
      backgroundOpacity: 100,
    },
    logging: {
      enabled: false,
      raw: false,
      text: true,
      jsonl: true,
      redactSecrets: true,
      pathTemplate: "{profile}/{date}/{session}.jsonl",
      retentionDays: 0,
    },
    triggers: [],
    transfer: {
      sftp: true,
      scp: true,
      xmodem: true,
      ymodem: true,
      zmodem: true,
      rateLimitBytesPerSecond: null,
      defaultLocalDir: null,
    },
  };
}

describe("SSH health Profile snapshots", () => {
  it("ignores Host Key mirrors and recorded authentication success", () => {
    const runtime = sshProfile();
    const current = structuredClone(runtime);
    if (current.connection.kind === "ssh") {
      const now = new Date().toISOString();
      current.connection.identityPolicy.lastSuccessful = "public-key";
      current.connection.trustedHostKeys.push({
        id: "host-key-1",
        profileId: current.id,
        alias: current.id,
        host: current.connection.endpoint.host,
        port: current.connection.endpoint.port,
        algorithm: "ssh-ed25519",
        fingerprintSha256: "SHA256:test",
        publicKeyBase64: "YWJj",
        scope: "profile",
        label: null,
        firstSeen: now,
        lastSeen: now,
      } satisfies TrustedHostKey);
    }

    expect(sshHealthProfileSnapshotMatches(runtime, current)).toBe(true);
  });

  it("invalidates endpoint, proxy, Jump Host, authentication, and terminal changes", () => {
    const runtime = sshProfile();
    const mutations = [
      (profile: SessionProfile) => {
        if (profile.connection.kind === "ssh") profile.connection.endpoint.host = "replacement.example";
      },
      (profile: SessionProfile) => {
        if (profile.connection.kind === "ssh") profile.connection.proxy.enabled = true;
      },
      (profile: SessionProfile) => {
        if (profile.connection.kind === "ssh") profile.connection.jumps[0].username = "replacement-user";
      },
      (profile: SessionProfile) => {
        if (profile.connection.kind === "ssh") profile.connection.identityPolicy.identitiesOnly = false;
      },
      (profile: SessionProfile) => {
        profile.terminal.rows += 1;
      },
    ];

    for (const mutate of mutations) {
      const current = structuredClone(runtime);
      mutate(current);
      expect(sshHealthProfileSnapshotMatches(runtime, current)).toBe(false);
    }
  });
});
