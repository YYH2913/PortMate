import { describe, expect, it } from "vitest";
import { createSshConnection } from "./session-profile-helpers";
import {
  cloneImportedProfile,
  parsePortMateProfileTransfer,
  serializePortMateProfileTransfer,
} from "./portmate-profile-transfer";
import type { SessionProfile } from "./types";

function profile(): SessionProfile {
  return {
    id: "profile-a",
    name: "Router",
    kind: "ssh",
    group: "",
    tags: [],
    connection: {
      ...createSshConnection(),
      identityRefs: [{
        id: "vault-key",
        label: "Production key",
        source: "profile-vault",
        fingerprintSha256: "SHA256:key",
        path: null,
        secretRef: "stronghold:key",
      }],
      passwordSecretRef: "stronghold:password",
      passphraseSecretRef: "stronghold:passphrase",
      jumps: [{
        host: "bastion",
        port: 22,
        username: "root",
        passwordSecretRef: "stronghold:jump-password",
        passphraseSecretRef: null,
        identityRef: "vault-key",
        hostKeyPolicy: null,
      }],
    },
    terminal: { term: "xterm-256color", rows: 24, cols: 80, scrollback: 2_000, fontFamily: "monospace", fontSize: 13, theme: "portmate-dark", backgroundOpacity: 100 },
    logging: { enabled: false, raw: false, text: true, jsonl: true, redactSecrets: true, pathTemplate: "{profile}/{date}/{session}.jsonl", retentionDays: 0 },
    triggers: [],
    transfer: { sftp: true, scp: true, tftp: true, xmodem: true, ymodem: true, zmodem: true, rateLimitBytesPerSecond: null, defaultLocalDir: null },
  };
}

describe("PortMate Profile transfer", () => {
  it("removes secret references and keeps a migration warning", () => {
    const parsed = parsePortMateProfileTransfer(serializePortMateProfileTransfer([profile()]));
    const connection = parsed.profiles[0].connection;
    expect(connection.kind).toBe("ssh");
    if (connection.kind !== "ssh") return;
    expect(connection.passwordSecretRef).toBeNull();
    expect(connection.identityRefs[0]).toMatchObject({ source: "public-key-only", secretRef: null });
    expect(connection.jumps[0].passwordSecretRef).toBeNull();
    expect(connection.jumps[0].identityRef).toBe(connection.identityRefs[0].id);
    expect(parsed.warnings.join(" ")).toContain("不会导出");
  });

  it("rejects malformed transfer documents and generates a new imported id", () => {
    expect(() => parsePortMateProfileTransfer({ format: "wrong", version: 1, profiles: [] })).toThrow();
    const parsed = parsePortMateProfileTransfer(serializePortMateProfileTransfer([profile()]));
    const imported = cloneImportedProfile(parsed.profiles[0], () => "new-id");
    expect(imported.id).toBe("new-id");
    if (imported.connection.kind === "ssh") {
      expect(imported.connection.identityRefs[0].id).toBe("identity-new-id");
      expect(imported.connection.jumps[0].identityRef).toBe("identity-new-id");
    }
  });

  it("strips secret references from hand-edited transfer documents", () => {
    const document = JSON.parse(serializePortMateProfileTransfer([profile()])) as Record<string, unknown>;
    const profiles = document.profiles as Array<Record<string, unknown>>;
    const connection = profiles[0].connection as Record<string, unknown>;
    connection.passwordSecretRef = "stronghold:local-password";
    const parsed = parsePortMateProfileTransfer(document);
    if (parsed.profiles[0].connection.kind !== "ssh") return;
    expect(parsed.profiles[0].connection.passwordSecretRef).toBeNull();
    expect(parsed.warnings.join(" " )).toContain("不会导出");
  });
});
