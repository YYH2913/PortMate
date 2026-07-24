import { describe, expect, it } from "vitest";
import {
  convertDraftProtocol,
  applyPuttyImportTerminal,
  createOpenSshImportConnection,
  createPuttyImportConnection,
  createShellImportConnection,
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

  it("translates OpenSSH import candidates into the existing SSH connection model", () => {
    const connection = createOpenSshImportConnection({
      id: "production",
      hostAlias: "production",
      host: "app.example.test",
      port: 2202,
      username: "deploy",
      hostKeyAlias: "production-device",
      identityFiles: ["~/.ssh/id_deploy", "~/.ssh/id_fallback"],
      keepaliveEnabled: true,
      keepaliveIntervalSeconds: 45,
      keepaliveMaxMissed: 5,
      tcpKeepaliveEnabled: false,
      identitiesOnly: false,
      forwardAgent: true,
      jumps: [{ host: "bastion.example.test", port: 2222, username: "ops" }],
      forwards: [
        { mode: "local", bindHost: "127.0.0.1", bindPort: 15432, targetHost: "db.example.test", targetPort: 5432 },
        { mode: "dynamic", bindHost: "127.0.0.1", bindPort: 1080, targetHost: "", targetPort: 0 },
      ],
      warnings: [],
    });

    expect(connection).toMatchObject({
      kind: "ssh",
      endpoint: { host: "app.example.test", port: 2202 },
      username: "deploy",
      keepaliveEnabled: true,
      keepaliveIntervalSeconds: 45,
      keepaliveMaxMissed: 5,
      tcpKeepaliveEnabled: false,
      hostKeyPolicy: { alias: "production-device" },
      identityPolicy: { identitiesOnly: false },
      agentPolicy: { forwarding: true },
      jumps: [{ host: "bastion.example.test", port: 2222, username: "ops" }],
      tunnels: [
        {
          id: "openssh-forward-1",
          label: "OpenSSH LocalForward 1",
          mode: "local",
          bindHost: "127.0.0.1",
          bindPort: 15432,
          targetHost: "db.example.test",
          targetPort: 5432,
          enabled: true,
        },
        {
          id: "openssh-forward-2",
          label: "OpenSSH DynamicForward 2",
          mode: "dynamic",
          bindHost: "127.0.0.1",
          bindPort: 1080,
          targetHost: "",
          targetPort: 0,
          enabled: true,
        },
      ],
    });
    expect(connection.identityRefs).toEqual(expect.arrayContaining([
      expect.objectContaining({ label: "id_deploy", source: "system-file", path: "~/.ssh/id_deploy" }),
      expect.objectContaining({ label: "id_fallback", source: "system-file", path: "~/.ssh/id_fallback" }),
    ]));
    expect(new Set(connection.identityRefs.map((identity) => identity.id)).size).toBe(2);
  });

  it("translates PuTTY network and serial candidates into supported connection models", () => {
    const ssh = createPuttyImportConnection({
      id: "putty-1-ops",
      name: "Ops",
      kind: "ssh",
      host: "ops.example.test",
      port: 2202,
      username: "operator",
      tryAgent: true,
      forwardAgent: true,
      keepaliveEnabled: false,
      keepaliveIntervalSeconds: 75,
      keepaliveMaxMissed: 0,
      tcpKeepaliveEnabled: true,
      proxy: { kind: "http-connect", host: "proxy.example.test", port: 8080, username: "relay" },
      forwards: [
        { mode: "local", bindHost: "127.0.0.1", bindPort: 15432, targetHost: "db.example.test", targetPort: 5432 },
        { mode: "dynamic", bindHost: "127.0.0.1", bindPort: 1080, targetHost: "", targetPort: 0 },
      ],
      warnings: [],
    });
    const serial = createPuttyImportConnection({
      id: "putty-2-bench",
      name: "Bench",
      kind: "serial",
      serial: {
        port: "/dev/ttyUSB0",
        baudRate: 115200,
        dataBits: 7,
        stopBits: 2,
        parity: "even",
        flowControl: "hardware",
      },
      warnings: [],
    });
    const raw = createPuttyImportConnection({
      id: "putty-3-raw",
      name: "Raw",
      kind: "tcp",
      host: "raw.example.test",
      port: 9000,
      username: "",
      tcpKeepaliveEnabled: false,
      warnings: [],
    });

    expect(ssh).toMatchObject({
      kind: "ssh",
      endpoint: { host: "ops.example.test", port: 2202 },
      username: "operator",
      keepaliveEnabled: false,
      keepaliveIntervalSeconds: 75,
      keepaliveMaxMissed: 0,
      tcpKeepaliveEnabled: true,
      proxy: { enabled: true, kind: "http-connect", host: "proxy.example.test", port: 8080, username: "relay" },
      agentPolicy: { enabled: true, forwarding: true },
      tunnels: [
        {
          id: "putty-forward-1",
          label: "PuTTY Local 1",
          mode: "local",
          bindHost: "127.0.0.1",
          bindPort: 15432,
          targetHost: "db.example.test",
          targetPort: 5432,
          enabled: true,
        },
        {
          id: "putty-forward-2",
          label: "PuTTY Dynamic 2",
          mode: "dynamic",
          bindHost: "127.0.0.1",
          bindPort: 1080,
          targetHost: "",
          targetPort: 0,
          enabled: true,
        },
      ],
    });
    expect(serial).toMatchObject({
      kind: "serial",
      port: "/dev/ttyUSB0",
      baudRate: 115200,
      dataBits: 7,
      stopBits: 2,
      parity: "even",
      flowControl: "hardware",
    });
    expect(raw).toMatchObject({ kind: "tcp", host: "raw.example.test", port: 9000, keepaliveEnabled: false });
  });

  it("applies imported PuTTY terminal settings without replacing unrelated profile defaults", () => {
    const terminal = applyPuttyImportTerminal(profile(createSshConnection()).terminal, {
      id: "putty-terminal",
      name: "Terminal",
      kind: "ssh",
      host: "terminal.example.test",
      port: 22,
      username: "operator",
      terminal: { term: "xterm-256color", rows: 40, cols: 180, scrollback: 5_000 },
      warnings: [],
    });

    expect(terminal).toMatchObject({
      term: "xterm-256color",
      rows: 40,
      cols: 180,
      scrollback: 5_000,
      fontFamily: "monospace",
      fontSize: 13,
    });
  });

  it("translates a discovered local Shell into the existing PTY connection model", () => {
    const connection = createShellImportConnection({
      id: "shell-1-/usr/bin/zsh",
      name: "zsh",
      program: "/usr/bin/zsh",
      args: [],
      warnings: [],
    });

    expect(connection).toEqual({ kind: "shell", program: "/usr/bin/zsh", args: [], cwd: null });
  });
});
