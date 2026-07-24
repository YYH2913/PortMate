import { describe, expect, it } from "vitest";
import {
  parseSessionSummaryCache,
  readSessionSummaryCache,
  SESSION_SUMMARY_CACHE_STORAGE_KEY,
} from "./session-summary-cache";
import type { ConnectionConfig, SessionKind, SessionSummary } from "./types";
import {
  MAX_TUNNELS_PER_PROFILE,
  MAX_TUNNEL_HOST_CHARACTERS,
} from "./tunnel-state";

describe("session summary cache", () => {
  it("reads legacy arrays and the versioned cache envelope", () => {
    const sessions = (["shell", "serial", "ssh", "tmux", "tcp", "telnet"] as const)
      .map((kind) => createSummary(`${kind}-a`, kind));
    expect(parseSessionSummaryCache(JSON.stringify(sessions))).toEqual(sessions);
    expect(parseSessionSummaryCache(JSON.stringify({ version: 1, sessions }))).toEqual(sessions);
  });

  it("migrates cached SSH profiles that predate the TCP keepalive preference", () => {
    const summary = createSummary("ssh-a", "ssh");
    if (summary.profile.connection.kind !== "ssh") throw new Error("expected SSH profile");
    delete (summary.profile.connection as { tcpKeepaliveEnabled?: boolean | null }).tcpKeepaliveEnabled;

    const [cached] = parseSessionSummaryCache(JSON.stringify([summary]));
    expect(cached.profile.connection).toMatchObject({ tcpKeepaliveEnabled: null });
  });

  it("rejects malformed, partial, inconsistent and duplicate snapshots as a whole", () => {
    const valid = createSummary("shell-a");
    expect(parseSessionSummaryCache("{")).toEqual([]);
    expect(parseSessionSummaryCache(JSON.stringify({}))).toEqual([]);
    expect(parseSessionSummaryCache(JSON.stringify([null]))).toEqual([]);
    expect(parseSessionSummaryCache(JSON.stringify([{ profile: { id: "partial" } }]))).toEqual([]);
    expect(parseSessionSummaryCache(JSON.stringify([{ ...valid, runtime: { ...valid.runtime, sessionId: "other" } }]))).toEqual([]);
    expect(parseSessionSummaryCache(JSON.stringify([valid, valid]))).toEqual([]);
    expect(parseSessionSummaryCache(JSON.stringify({ version: 2, sessions: [valid] }))).toEqual([]);
  });

  it("rejects incomplete protocol data before a cached window can render it", () => {
    const serial = createSummary("serial-a", "serial");
    const ssh = createSummary("ssh-a", "ssh");
    expect(parseSessionSummaryCache(JSON.stringify([serial, ssh]))).toEqual([serial, ssh]);
    expect(parseSessionSummaryCache(JSON.stringify([
      { ...serial, profile: { ...serial.profile, connection: { kind: "serial", port: "/dev/ttyUSB0" } } },
    ]))).toEqual([]);
    expect(parseSessionSummaryCache(JSON.stringify([
      { ...ssh, profile: { ...ssh.profile, connection: { ...ssh.profile.connection, endpoint: null } } },
    ]))).toEqual([]);
  });

  it("rejects trigger collections and payloads outside runtime bounds", () => {
    const summary = createSummary("shell-a");
    const trigger = {
      id: "trigger-a",
      label: "Trigger",
      matcher: { type: "contains" as const, text: "match", case_sensitive: true },
      actions: [{ type: "timeline-mark" as const, label: "mark" }],
      enabled: true,
    };
    summary.profile.triggers = Array.from({ length: 65 }, (_, index) => ({
      ...trigger,
      id: `trigger-${index}`,
    }));
    expect(parseSessionSummaryCache(JSON.stringify([summary]))).toEqual([]);

    summary.profile.triggers = [{
      ...trigger,
      actions: [{ type: "local-command", command: "x".repeat(4_097) }],
    }];
    expect(parseSessionSummaryCache(JSON.stringify([summary]))).toEqual([]);

    summary.profile.triggers = [trigger, { ...trigger }];
    expect(parseSessionSummaryCache(JSON.stringify([summary]))).toEqual([]);
  });

  it("rejects tunnel collections and fields outside backend bounds", () => {
    const summary = createSummary("ssh-a", "ssh");
    if (summary.profile.connection.kind !== "ssh") throw new Error("expected SSH profile");
    const tunnel = {
      id: "tunnel-a",
      label: "Tunnel A",
      mode: "local" as const,
      bindHost: "127.0.0.1",
      bindPort: 10_022,
      targetHost: "device.internal",
      targetPort: 22,
      enabled: true,
    };
    summary.profile.connection.tunnels = Array.from(
      { length: MAX_TUNNELS_PER_PROFILE + 1 },
      (_, index) => ({ ...tunnel, id: `tunnel-${index}` }),
    );
    expect(parseSessionSummaryCache(JSON.stringify([summary]))).toEqual([]);

    summary.profile.connection.tunnels = [
      tunnel,
      { ...tunnel, label: "Duplicate" },
    ];
    expect(parseSessionSummaryCache(JSON.stringify([summary]))).toEqual([]);

    summary.profile.connection.tunnels = [{
      ...tunnel,
      targetHost: "x".repeat(MAX_TUNNEL_HOST_CHARACTERS + 1),
    }];
    expect(parseSessionSummaryCache(JSON.stringify([summary]))).toEqual([]);

    summary.profile.connection.tunnels = [{
      ...tunnel,
      mode: "dynamic",
      targetHost: "must-be-empty.invalid",
      targetPort: 443,
    }];
    expect(parseSessionSummaryCache(JSON.stringify([summary]))).toEqual([]);

    summary.profile.connection.tunnels = [{ ...tunnel, bindPort: 1.5 }];
    expect(parseSessionSummaryCache(JSON.stringify([summary]))).toEqual([]);
  });

  it("treats storage failures as a non-authoritative cache miss", () => {
    const sessions = [createSummary("shell-a")];
    expect(readSessionSummaryCache({
      getItem: (key) => key === SESSION_SUMMARY_CACHE_STORAGE_KEY
        ? JSON.stringify({ version: 1, sessions })
        : null,
    })).toEqual(sessions);
    expect(readSessionSummaryCache({ getItem: () => { throw new Error("denied"); } })).toEqual([]);
  });

  it("normalizes legacy disconnect reasons before cached windows consume them", () => {
    const summary = createSummary("shell-a");
    summary.runtime.lastDisconnectReason = `  socket\n\tclosed ${"界".repeat(300)}  `;
    const [cached] = parseSessionSummaryCache(JSON.stringify([summary]));

    expect(cached.runtime.lastDisconnectReason?.startsWith("socket closed 界")).toBe(true);
    expect(cached.runtime.lastDisconnectReason?.endsWith("...")).toBe(true);
    expect(cached.runtime.lastDisconnectReason).not.toContain("\n");
    expect(Array.from(cached.runtime.lastDisconnectReason ?? "")).toHaveLength(256);

    summary.runtime.lastDisconnectReason = " \n\t ";
    expect(parseSessionSummaryCache(JSON.stringify([summary]))[0].runtime.lastDisconnectReason).toBeNull();
  });
});

function createSummary(id: string, kind: SessionKind = "shell"): SessionSummary {
  return {
    profile: {
      id,
      name: id,
      kind,
      group: "",
      tags: [],
      connection: createConnection(kind),
      terminal: { term: "xterm-256color", rows: 32, cols: 120, scrollback: 200000, fontFamily: "monospace", fontSize: 13, theme: "portmate-dark", backgroundOpacity: 100 },
      logging: { enabled: false, raw: false, text: false, jsonl: false, redactSecrets: true, pathTemplate: "{profile}.jsonl", retentionDays: 0 },
      triggers: [],
      transfer: { sftp: false, scp: false, xmodem: false, ymodem: false, zmodem: false, rateLimitBytesPerSecond: null, defaultLocalDir: null },
    },
    runtime: {
      sessionId: id,
      paneId: `${id}:main`,
      status: "disconnected",
      title: id,
      cwd: null,
      connectedSince: null,
      lastActivity: new Date(0).toISOString(),
      lastDisconnect: null,
      lastDisconnectReason: null,
      activeTransport: kind,
    },
    logLines: 0,
    lastLine: null,
  };
}

function createConnection(kind: SessionKind): ConnectionConfig {
  switch (kind) {
    case "shell":
      return { kind, program: "/bin/sh", args: [], cwd: null };
    case "serial":
      return {
        kind,
        port: "/dev/ttyUSB0",
        baudRate: 115200,
        dataBits: 8,
        stopBits: 1,
        parity: "none",
        flowControl: "none",
        dtr: false,
        rts: false,
        reconnect: true,
        reconnectDelayMs: 1000,
        receiveIdleTimeoutEnabled: false,
        receiveIdleTimeoutSeconds: 60,
      };
    case "tcp":
    case "telnet":
      return {
        kind,
        host: "example.test",
        port: kind === "telnet" ? 23 : 9000,
        reconnect: true,
        proxy: { enabled: false, kind: "socks5", host: "127.0.0.1", port: 1080, username: "", passwordSecretRef: null },
        reconnectDelayMs: 1000,
        keepaliveEnabled: true,
        keepaliveIdleSeconds: 30,
        keepaliveIntervalSeconds: 10,
        keepaliveRetries: 3,
        telnetBinary: false,
        telnetNaws: true,
      };
    case "ssh":
    case "tmux":
      return {
        kind,
        endpoint: { host: "example.test", port: 22 },
        username: "operator",
        reconnect: true,
        reconnectDelayMs: 1000,
        keepaliveEnabled: true,
        keepaliveIntervalSeconds: 30,
        keepaliveMaxMissed: 3,
        tcpKeepaliveEnabled: null,
        proxy: { enabled: false, kind: "socks5" as const, host: "127.0.0.1", port: 1080, username: "", passwordSecretRef: null },
        passwordSecretRef: null,
        passphraseSecretRef: null,
        hostKeyPolicy: { mode: "trust-on-first-use" as const, alias: null, trustScope: "profile" as const, allowRotation: false, checkIp: false },
        trustedHostKeys: [],
        identityPolicy: { identitiesOnly: true, authOrder: ["public-key" as const], recordSuccess: true, lastSuccessful: null },
        identityRefs: [],
        agentPolicy: { enabled: false, forwarding: false, offerMode: "after-profile-keys" as const },
        jumps: [],
        tunnels: [],
      };
  }
}
