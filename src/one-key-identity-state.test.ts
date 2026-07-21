import { describe, expect, it } from "vitest";
import {
  oneKeyIdentityCandidates,
  oneKeyIdentitySelectionKey,
  oneKeyIdentityUpdate,
  selectionFromOneKeyIdentity,
} from "./one-key-identity-state";
import type { ConnectionConfig, IdentityRef, OneKeyIdentitySummary, SessionSummary } from "./types";

function identity(id: string, source: IdentityRef["source"]): IdentityRef {
  return { id, label: id, source };
}

function session(
  id: string,
  kind: "ssh" | "tmux" | "shell",
  identities: IdentityRef[] = [],
): SessionSummary {
  const connection: ConnectionConfig = kind === "shell"
    ? { kind: "shell" as const, program: "/bin/sh", args: [] }
    : {
      kind,
      endpoint: { host: "example.test", port: 22 },
      username: "operator",
      reconnect: false,
      reconnectDelayMs: 1_000,
      keepaliveEnabled: true,
      keepaliveIntervalSeconds: 30,
      keepaliveMaxMissed: 3,
      proxy: { enabled: false, kind: "socks5" as const, host: "", port: 1080, username: "" },
      hostKeyPolicy: {
        mode: "strict" as const,
        trustScope: "profile" as const,
        allowRotation: false,
        checkIp: false,
      },
      trustedHostKeys: [],
      identityPolicy: {
        identitiesOnly: true,
        authOrder: ["public-key" as const],
        recordSuccess: true,
      },
      identityRefs: identities,
      agentPolicy: {
        enabled: true,
        forwarding: false,
        offerMode: "after-profile-keys" as const,
      },
      jumps: [],
      tunnels: [],
    };
  return {
    profile: {
      id,
      name: id,
      kind,
      group: "",
      tags: [],
      connection,
      terminal: {
        term: "xterm-256color",
        rows: 24,
        cols: 80,
        scrollback: 1_000,
        fontFamily: "monospace",
        fontSize: 13,
        theme: "default",
        backgroundOpacity: 100,
      },
      logging: {
        enabled: false,
        raw: false,
        text: false,
        jsonl: false,
        redactSecrets: true,
        pathTemplate: "",
        retentionDays: 30,
      },
      triggers: [],
      transfer: { sftp: false, scp: false, xmodem: false, ymodem: false, zmodem: false },
    },
    runtime: {
      sessionId: id,
      status: "disconnected",
      paneId: id,
      title: id,
      lastActivity: "2026-07-15T00:00:00Z",
      activeTransport: kind,
    },
    logLines: 0,
  };
}

describe("OneKey public-key identity state", () => {
  it("offers authenticating identities only from bound SSH/Tmux profiles", () => {
    const sessions = [
      session("ssh-a", "ssh", [identity("vault", "profile-vault"), identity("public", "public-key-only")]),
      session("tmux-b", "tmux", [identity("agent", "agent")]),
      session("ssh-c", "ssh", [identity("file", "system-file")]),
      session("shell-d", "shell"),
    ];

    expect(oneKeyIdentityCandidates(sessions, ["ssh-a", "tmux-b", "shell-d"]).map((item) => item.identity.id))
      .toEqual(["vault", "agent"]);
  });

  it("preserves the same identity, sets a different one, and clears empty selections", () => {
    const current: OneKeyIdentitySummary = {
      sourceProfileId: "ssh-a",
      id: "vault",
      label: "Vault key",
      source: "profile-vault",
    };
    const selected = selectionFromOneKeyIdentity(current);

    expect(oneKeyIdentityUpdate("ssh", current, selected)).toEqual({ action: "preserve" });
    expect(oneKeyIdentityUpdate("ssh", current, { sourceProfileId: "tmux-b", identityId: "agent" }))
      .toEqual({ action: "set", sourceProfileId: "tmux-b", identityId: "agent" });
    expect(oneKeyIdentityUpdate("ssh", current, null)).toEqual({ action: "clear" });
    expect(oneKeyIdentityUpdate("account", current, selected)).toEqual({ action: "clear" });
    expect(oneKeyIdentitySelectionKey(selected!)).toBe('["ssh-a","vault"]');
  });
});
