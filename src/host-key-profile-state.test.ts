import { describe, expect, it } from "vitest";
import { hostKeyProfileSnapshotMatches } from "./host-key-profile-state";
import { createSshConnection } from "./session-profile-helpers";
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

describe("Host Key Profile snapshots", () => {
  it("ignores persistent Host Key mirrors", () => {
    const scanned = sshProfile();
    const current = structuredClone(scanned);
    if (current.connection.kind === "ssh") {
      const now = new Date().toISOString();
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

    expect(hostKeyProfileSnapshotMatches(scanned, current)).toBe(true);
  });

  it("invalidates snapshots when the endpoint or authentication route changes", () => {
    const scanned = sshProfile();
    const changedEndpoint = structuredClone(scanned);
    if (changedEndpoint.connection.kind === "ssh") {
      changedEndpoint.connection.endpoint.host = "replacement.example";
    }
    const changedJump = structuredClone(scanned);
    if (changedJump.connection.kind === "ssh") {
      changedJump.connection.jumps[0].username = "replacement-user";
    }

    expect(hostKeyProfileSnapshotMatches(scanned, changedEndpoint)).toBe(false);
    expect(hostKeyProfileSnapshotMatches(scanned, changedJump)).toBe(false);
  });
});
