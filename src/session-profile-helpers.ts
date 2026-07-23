import { proxyDefaults } from "./proxy-settings";
import { serialConnectionDefaults } from "./serial-connection-settings";
import type { ProtocolTab } from "./session-settings-state";
import { sshConnectionDefaults } from "./ssh-connection-settings";
import { tcpConnectionDefaults } from "./tcp-connection-settings";
import type { OpenSshImportCandidate } from "./openssh-config-import";
import type { PuttySessionImportCandidate } from "./putty-session-import";
import type { ShellSessionImportCandidate } from "./shell-session-import";
import type {
  ConnectionConfig,
  HostKeyPolicy,
  HostKeyScanResult,
  IdentityRef,
  JumpHop,
  SessionKind,
  SessionProfile,
  TriggerSpec,
} from "./types";

export function createSerialConnection(): Extract<ConnectionConfig, { kind: "serial" }> {
  return {
    kind: "serial",
    port: "",
    baudRate: 115200,
    dataBits: 8,
    stopBits: 1,
    parity: "none",
    flowControl: "none",
    dtr: false,
    rts: false,
    reconnect: true,
    ...serialConnectionDefaults,
  };
}

export function createShellConnection(): Extract<ConnectionConfig, { kind: "shell" }> {
  return {
    kind: "shell",
    program: "",
    args: [],
    cwd: null,
  };
}

export function createTcpConnection(kind: "telnet" | "tcp"): Extract<ConnectionConfig, { kind: "telnet" | "tcp" }> {
  return {
    kind,
    host: "",
    port: kind === "telnet" ? 23 : 0,
    reconnect: true,
    proxy: { ...proxyDefaults },
    ...tcpConnectionDefaults,
  };
}

export function createSshConnection(): Extract<ConnectionConfig, { kind: "ssh" | "tmux" }> {
  return {
    kind: "ssh",
    endpoint: { host: "", port: 22 },
    username: "",
    reconnect: true,
    ...sshConnectionDefaults,
    proxy: { ...proxyDefaults },
    passwordSecretRef: null,
    passphraseSecretRef: null,
    hostKeyPolicy: {
      mode: "trust-on-first-use",
      alias: null,
      trustScope: "profile",
      allowRotation: false,
      checkIp: false,
    },
    trustedHostKeys: [],
    identityPolicy: {
      identitiesOnly: true,
      authOrder: ["public-key", "keyboard-interactive", "password"],
      recordSuccess: true,
      lastSuccessful: null,
    },
    identityRefs: [],
    agentPolicy: {
      enabled: false,
      forwarding: false,
      offerMode: "after-profile-keys",
    },
    jumps: [],
    tunnels: [],
  };
}

export function createOpenSshImportConnection(candidate: OpenSshImportCandidate): Extract<ConnectionConfig, { kind: "ssh" | "tmux" }> {
  const connection = createSshConnection();
  return {
    ...connection,
    kind: "ssh",
    endpoint: { host: candidate.host, port: candidate.port },
    username: candidate.username,
    keepaliveEnabled: candidate.keepaliveEnabled ?? connection.keepaliveEnabled,
    keepaliveIntervalSeconds: candidate.keepaliveIntervalSeconds ?? connection.keepaliveIntervalSeconds,
    keepaliveMaxMissed: candidate.keepaliveMaxMissed ?? connection.keepaliveMaxMissed,
    hostKeyPolicy: { ...connection.hostKeyPolicy, alias: candidate.hostKeyAlias ?? candidate.hostAlias },
    identityPolicy: {
      ...connection.identityPolicy,
      identitiesOnly: candidate.identitiesOnly ?? connection.identityPolicy.identitiesOnly,
    },
    identityRefs: candidate.identityFiles.map((path, index) => ({
      ...createIdentityRef(),
      label: importedIdentityLabel(path, index),
      source: "system-file",
      path,
      secretRef: null,
    })),
    agentPolicy: {
      ...connection.agentPolicy,
      forwarding: candidate.forwardAgent ?? connection.agentPolicy.forwarding,
    },
    jumps: candidate.jumps.map((jump) => ({
      host: jump.host,
      port: jump.port,
      username: jump.username || candidate.username,
      passwordSecretRef: null,
      passphraseSecretRef: null,
      identityRef: null,
      hostKeyPolicy: null,
    })),
    tunnels: candidate.forwards.map((forward, index) => ({
      id: `openssh-forward-${index + 1}`,
      label: `OpenSSH ${openSshForwardLabel(forward.mode)} ${index + 1}`,
      mode: forward.mode,
      bindHost: forward.bindHost,
      bindPort: forward.bindPort,
      targetHost: forward.targetHost,
      targetPort: forward.targetPort,
      enabled: true,
    })),
  };
}

function openSshForwardLabel(mode: "local" | "remote" | "dynamic"): string {
  if (mode === "local") return "LocalForward";
  if (mode === "remote") return "RemoteForward";
  return "DynamicForward";
}

export function createPuttyImportConnection(candidate: PuttySessionImportCandidate): ConnectionConfig {
  if (candidate.kind === "serial") {
    const connection = createSerialConnection();
    return {
      ...connection,
      kind: "serial",
      ...candidate.serial,
    };
  }

  if (candidate.kind === "ssh") {
    const connection = createSshConnection();
    return {
      ...connection,
      kind: "ssh",
      endpoint: { host: candidate.host, port: candidate.port },
      username: candidate.username,
      proxy: candidate.proxy
        ? { ...connection.proxy, ...candidate.proxy, enabled: true }
        : connection.proxy,
      agentPolicy: {
        ...connection.agentPolicy,
        enabled: candidate.tryAgent ?? connection.agentPolicy.enabled,
        forwarding: candidate.forwardAgent ?? connection.agentPolicy.forwarding,
      },
    };
  }

  const connection = createTcpConnection(candidate.kind);
  return {
    ...connection,
    kind: candidate.kind,
    host: candidate.host,
    port: candidate.port,
    proxy: candidate.proxy
      ? { ...connection.proxy, ...candidate.proxy, enabled: true }
      : connection.proxy,
  };
}

export function createShellImportConnection(candidate: ShellSessionImportCandidate): Extract<ConnectionConfig, { kind: "shell" }> {
  return {
    ...createShellConnection(),
    kind: "shell",
    program: candidate.program,
    args: [...candidate.args],
  };
}

function importedIdentityLabel(path: string, index: number) {
  const parts = path.replaceAll("\\", "/").split("/").filter(Boolean);
  return parts[parts.length - 1] || `identity ${index + 1}`;
}

export function createIdentityRef(): IdentityRef {
  return {
    id: `identity-${typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : Date.now()}`,
    label: "profile key",
    source: "system-file",
    fingerprintSha256: null,
    path: null,
    secretRef: null,
  };
}

export function serialPortOptions(current: string, discovered: string[]) {
  return Array.from(new Set([current, ...discovered, "COM1", "COM2", "COM3", "COM7", "/dev/ttyUSB0", "/dev/ttyACM0"].filter(Boolean)));
}

export function formatSshTarget(ssh: Extract<ConnectionConfig, { kind: "ssh" | "tmux" }>) {
  return ssh.username ? `${ssh.username}@${ssh.endpoint.host}` : ssh.endpoint.host;
}

export function parseSshTarget(value: string, ssh: Extract<ConnectionConfig, { kind: "ssh" | "tmux" }>): Extract<ConnectionConfig, { kind: "ssh" | "tmux" }> {
  const at = value.lastIndexOf("@");
  if (at > 0) {
    return { ...ssh, kind: "ssh", username: value.slice(0, at), endpoint: { ...ssh.endpoint, host: value.slice(at + 1) } };
  }
  return { ...ssh, kind: "ssh", endpoint: { ...ssh.endpoint, host: value } };
}

export function protocolFromKind(kind: SessionKind): ProtocolTab {
  switch (kind) {
    case "shell":
      return "Shell";
    case "ssh":
      return "SSH";
    case "tmux":
      return "Tmux";
    case "telnet":
      return "Telnet";
    case "tcp":
      return "Tcp";
    case "serial":
      return "Serial";
  }
}

export function convertDraftProtocol(draft: SessionProfile, protocol: ProtocolTab): SessionProfile {
  switch (protocol) {
    case "Shell":
      return { ...draft, kind: "shell", connection: draft.connection.kind === "shell" ? draft.connection : createShellConnection() };
    case "SSH":
      return { ...draft, kind: "ssh", connection: draft.connection.kind === "ssh" || draft.connection.kind === "tmux" ? { ...draft.connection, kind: "ssh" } : createSshConnection() };
    case "Tmux":
      return { ...draft, kind: "tmux", connection: draft.connection.kind === "ssh" || draft.connection.kind === "tmux" ? { ...draft.connection, kind: "tmux" } : { ...createSshConnection(), kind: "tmux" } };
    case "Telnet":
      return { ...draft, kind: "telnet", connection: draft.connection.kind === "telnet" ? draft.connection : createTcpConnection("telnet") };
    case "Tcp":
      return { ...draft, kind: "tcp", connection: draft.connection.kind === "tcp" ? draft.connection : createTcpConnection("tcp") };
    case "Serial":
      return { ...draft, kind: "serial", connection: draft.connection.kind === "serial" ? draft.connection : createSerialConnection() };
  }
}

export function createJumpHostKeyPolicy(jump?: JumpHop): HostKeyPolicy {
  const host = jump?.host.trim();
  const port = jump?.port && Number.isFinite(jump.port) ? Math.trunc(jump.port) : 22;
  return {
    mode: "trust-on-first-use",
    alias: host ? `jump:${host}:${port}` : null,
    trustScope: "profile",
    allowRotation: false,
    checkIp: false,
  };
}

export function createDefaultTrigger(): TriggerSpec {
  return {
    id: `trigger-${typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : Date.now()}`,
    label: "关键输出",
    matcher: { type: "contains", text: "error", case_sensitive: false },
    actions: [{ type: "timeline-mark", label: "error" }],
    enabled: true,
  };
}

export function profileCredentialSecretRefs(profile: SessionProfile): Set<string> {
  const refs = new Set<string>();
  const add = (secretRef?: string | null) => {
    if (secretRef) refs.add(secretRef);
  };
  const connection = profile.connection;
  if (connection.kind === "ssh" || connection.kind === "tmux") {
    add(connection.proxy.passwordSecretRef);
    add(connection.passwordSecretRef);
    add(connection.passphraseSecretRef);
    for (const identity of connection.identityRefs) add(identity.secretRef);
    for (const jump of connection.jumps) {
      add(jump.passwordSecretRef);
      add(jump.passphraseSecretRef);
    }
  } else if (connection.kind === "tcp" || connection.kind === "telnet") {
    add(connection.proxy.passwordSecretRef);
  }
  return refs;
}

export function describeHostKeyEvaluation(result: HostKeyScanResult) {
  const evaluation = result.evaluation;
  const prefix = result.label ? `${result.label}: ` : "";
  if (evaluation.status === "trusted") {
    return `${prefix}已信任 ${evaluation.fingerprintSha256}`;
  }
  if (evaluation.status === "mismatch") {
    return `${prefix}不匹配 ${evaluation.algorithm} ${evaluation.observedFingerprintSha256}`;
  }
  return `${prefix}未知 ${evaluation.algorithm} ${evaluation.fingerprintSha256}`;
}
