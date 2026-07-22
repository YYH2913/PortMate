import { describe, expect, it } from "vitest";
import {
  convertDraftProtocol,
  createSerialConnection,
  createSshConnection,
  formatSshTarget,
  parseSshTarget,
  profileCredentialSecretRefs,
  serialPortOptions,
} from "./session-profile-helpers";
import type { SessionProfile } from "./types";

function profile(connection: SessionProfile["connection"]): SessionProfile {
  return {
    id: "profile-1",
    name: "Profile",
    kind: connection.kind,
    group: "",
    tags: [],
    connection,
    terminal: {
      term: "xterm-256color",
      rows: 32,
      cols: 120,
      scrollback: 2_000,
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
      pathTemplate: "",
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

describe("session profile helpers", () => {
  it("preserves SSH settings when switching between SSH and Tmux", () => {
    const ssh = createSshConnection();
    ssh.endpoint.host = "router.example";
    ssh.username = "ops";
    const draft = profile(ssh);

    const tmux = convertDraftProtocol(draft, "Tmux");
    expect(tmux.kind).toBe("tmux");
    expect(tmux.connection).toMatchObject({ kind: "tmux", endpoint: { host: "router.example" }, username: "ops" });

    const restored = convertDraftProtocol(tmux, "SSH");
    expect(restored.connection).toMatchObject({ kind: "ssh", endpoint: { host: "router.example" }, username: "ops" });
  });

  it("uses protocol defaults when switching incompatible transports", () => {
    const serial = profile(createSerialConnection());
    expect(convertDraftProtocol(serial, "Telnet").connection).toMatchObject({ kind: "telnet", port: 23 });
    expect(convertDraftProtocol(serial, "Tcp").connection).toMatchObject({ kind: "tcp", port: 0 });
    expect(convertDraftProtocol(serial, "Shell").connection).toMatchObject({ kind: "shell", args: [] });
  });

  it("collects every credential reference without empty values", () => {
    const ssh = createSshConnection();
    ssh.proxy.passwordSecretRef = "proxy-secret";
    ssh.passwordSecretRef = "password-secret";
    ssh.passphraseSecretRef = "passphrase-secret";
    ssh.identityRefs = [{
      id: "identity-1",
      label: "Identity",
      source: "profile-vault",
      fingerprintSha256: null,
      path: null,
      secretRef: "identity-secret",
    }];
    ssh.jumps = [{
      host: "jump.example",
      port: 22,
      username: "jump",
      passwordSecretRef: "jump-password",
      passphraseSecretRef: null,
      identityRef: null,
      hostKeyPolicy: null,
    }];

    expect([...profileCredentialSecretRefs(profile(ssh))].sort()).toEqual([
      "identity-secret",
      "jump-password",
      "passphrase-secret",
      "password-secret",
      "proxy-secret",
    ]);
  });

  it("parses SSH targets and deduplicates serial port suggestions", () => {
    const ssh = createSshConnection();
    const parsed = parseSshTarget("ops@router.example", ssh);
    expect(formatSshTarget(parsed)).toBe("ops@router.example");
    expect(serialPortOptions("/dev/ttyUSB0", ["/dev/ttyUSB0", "/dev/ttyACM0"]))
      .toEqual(expect.arrayContaining(["/dev/ttyUSB0", "/dev/ttyACM0", "COM1"]));
    expect(serialPortOptions("/dev/ttyUSB0", ["/dev/ttyUSB0"]).filter((port) => port === "/dev/ttyUSB0"))
      .toHaveLength(1);
  });
});
