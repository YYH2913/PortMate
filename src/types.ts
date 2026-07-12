export type SessionKind = "ssh" | "serial" | "shell" | "telnet" | "tcp" | "tmux";
export type SessionStatus = "disconnected" | "connecting" | "connected" | "reconnecting" | "blocked" | "error";
export type EventDirection = "inbound" | "outbound" | "system";
export type AuthMethod = "public-key" | "keyboard-interactive" | "password" | "gssapi-with-mic" | "none";

export interface SessionSummary {
  profile: SessionProfile;
  runtime: SessionRuntime;
  logLines: number;
  lastLine?: string | null;
}

export interface SessionProfile {
  id: string;
  name: string;
  kind: SessionKind;
  group: string;
  tags: string[];
  connection: ConnectionConfig;
  terminal: {
    term: string;
    rows: number;
    cols: number;
    scrollback: number;
    fontFamily: string;
    fontSize: number;
    theme: string;
  };
  logging: {
    enabled: boolean;
    raw: boolean;
    text: boolean;
    jsonl: boolean;
    redactSecrets: boolean;
    pathTemplate: string;
    retentionDays: number;
  };
  triggers: TriggerSpec[];
  transfer: {
    sftp: boolean;
    scp: boolean;
    xmodem: boolean;
    ymodem: boolean;
    zmodem: boolean;
    rateLimitBytesPerSecond?: number | null;
    defaultLocalDir?: string | null;
  };
}

export type ConnectionConfig =
  | ({ kind: "ssh" | "tmux" } & SshConnection)
  | ({ kind: "serial" } & SerialConnection)
  | ({ kind: "shell" } & ShellConnection)
  | ({ kind: "telnet" | "tcp" } & TcpConnection);

export interface HostKeyPolicy {
  mode: "strict" | "trust-on-first-use" | "ask-every-time";
  alias?: string | null;
  trustScope: "profile" | "project" | "user";
  allowRotation: boolean;
  checkIp: boolean;
}

export interface SshConnection {
  endpoint: { host: string; port: number };
  username: string;
  reconnect: boolean;
  passwordSecretRef?: string | null;
  passphraseSecretRef?: string | null;
  hostKeyPolicy: HostKeyPolicy;
  trustedHostKeys: TrustedHostKey[];
  identityPolicy: {
    identitiesOnly: boolean;
    authOrder: AuthMethod[];
    recordSuccess: boolean;
    lastSuccessful?: AuthMethod | null;
  };
  identityRefs: IdentityRef[];
  agentPolicy: {
    enabled: boolean;
    forwarding: boolean;
    offerMode: "disabled" | "after-profile-keys" | "before-profile-keys";
  };
  jumps: JumpHop[];
  tunnels: TunnelSpec[];
}

export interface JumpHop {
  host: string;
  port: number;
  username: string;
  passwordSecretRef?: string | null;
  passphraseSecretRef?: string | null;
  identityRef?: string | null;
  hostKeyPolicy?: HostKeyPolicy | null;
}

export interface SerialConnection {
  port: string;
  baudRate: number;
  dataBits: number;
  stopBits: number;
  parity: string;
  flowControl: string;
  dtr: boolean;
  rts: boolean;
  reconnect: boolean;
}

export interface ShellConnection {
  program: string;
  args: string[];
  cwd?: string | null;
}

export interface TcpConnection {
  host: string;
  port: number;
  reconnect: boolean;
}

export interface TrustedHostKey {
  id: string;
  profileId?: string | null;
  alias: string;
  host: string;
  port: number;
  algorithm: string;
  fingerprintSha256: string;
  publicKeyBase64: string;
  scope: "profile" | "project" | "user";
  label?: string | null;
  firstSeen: string;
  lastSeen: string;
}

export interface IdentityRef {
  id: string;
  label: string;
  source: "profile-vault" | "system-file" | "agent" | "public-key-only";
  fingerprintSha256?: string | null;
  path?: string | null;
  secretRef?: string | null;
}

export interface TunnelSpec {
  id: string;
  label: string;
  mode: "local" | "remote" | "dynamic";
  bindHost: string;
  bindPort: number;
  targetHost: string;
  targetPort: number;
  enabled: boolean;
}

export interface TunnelStatus {
  spec: TunnelSpec;
  activeConnections: number;
  totalConnections: number;
  tcpToSshBytes: number;
  sshToTcpBytes: number;
  lastActivity?: string | null;
  lastError?: string | null;
}

export interface TriggerSpec {
  id: string;
  label: string;
  matcher: Record<string, unknown>;
  actions: Record<string, unknown>[];
  enabled: boolean;
}

export interface SessionRuntime {
  sessionId: string;
  paneId: string;
  status: SessionStatus;
  title: string;
  cwd?: string | null;
  connectedSince?: string | null;
  lastActivity: string;
  lastDisconnect?: string | null;
  lastDisconnectReason?: string | null;
  activeTransport: SessionKind;
}

export interface SessionEvent {
  id: string;
  sessionId: string;
  paneId: string;
  ts: string;
  direction: EventDirection;
  stream: "stdout" | "stderr" | "control" | "audit";
  bytesRef?: string | null;
  text?: string | null;
  annotations: Record<string, string>;
}

export interface LogShardInfo {
  path: string;
  format: "raw" | "txt" | "jsonl";
  size: number;
  modifiedAt?: string | null;
}

export interface LogShardPreview {
  path: string;
  content: string;
  encoding: "utf8" | "hex";
  bytesRead: number;
  truncated: boolean;
}

export interface DeleteLogShardsResult {
  deleted: number;
  bytesDeleted: number;
}

export interface ExportSessionBundleArchiveResult {
  path: string;
  checksumPath: string;
  sha256: string;
  size: number;
  files: number;
  rawLogSegments: number;
  redacted: boolean;
  warnings: string[];
}

export interface LogShardSearchMatch {
  path: string;
  format: "txt" | "jsonl";
  line: number;
  byteOffset: number;
  text: string;
}

export interface SearchLogShardsResult {
  matches: LogShardSearchMatch[];
  filesScanned: number;
  bytesScanned: number;
  truncated: boolean;
  warnings: string[];
}

export interface ArchiveLogShardsResult {
  path: string;
  checksumPath: string;
  sha256: string;
  size: number;
  shards: number;
  sourceBytes: number;
}

export interface TransferTask {
  id: string;
  sessionId: string;
  protocol: "sftp" | "scp" | "xmodem" | "ymodem" | "zmodem";
  source: string;
  destination: string;
  bytesTotal: number;
  bytesDone: number;
  status: "queued" | "running" | "completed" | "failed" | "cancelled";
  message?: string | null;
  startedAt?: string | null;
  finishedAt?: string | null;
  averageBytesPerSecond?: number | null;
}

export interface SysmonSnapshot {
  sessionId: string;
  ts: string;
  uptimeSeconds: number;
  cpuPercent: number;
  memoryPercent: number;
  rxKbps: number;
  txKbps: number;
}

export interface AuditRecord {
  id: string;
  ts: string;
  actor: string;
  action: string;
  sessionId?: string | null;
  decision: string;
  details: Record<string, string>;
}

export type McpScope = "read-sessions" | "read-logs" | "write-input" | "transfer" | "tunnel" | "manage-sessions";

export interface McpGrant {
  clientId: string;
  name: string;
  scopes: McpScope[];
  allowedSessions: string[];
  expiresAt?: string | null;
  revokedAt?: string | null;
}

export interface McpHttpConfig {
  endpoint: string;
  tokenRef: string;
  tokenAvailable: boolean;
  defaultOrigin: string;
  startCommand: string;
}

export interface McpHttpTokenResponse {
  config: McpHttpConfig;
  token: string;
}

export interface HostKeyStore {
  keys: TrustedHostKey[];
}

export type HostKeyEvaluation =
  | { status: "trusted"; matchedKeyId: string; fingerprintSha256: string }
  | { status: "unknown"; alias: string; host: string; port: number; algorithm: string; fingerprintSha256: string; reason: string }
  | { status: "mismatch"; alias: string; host: string; port: number; algorithm: string; expected: TrustedHostKey[]; observedFingerprintSha256: string; reason: string };

export interface HostKeyObservation {
  host: string;
  port: number;
  alias?: string | null;
  algorithm: string;
  publicKeyBase64: string;
}

export interface HostKeyScanResult {
  label?: string | null;
  observation: HostKeyObservation;
  evaluation: HostKeyEvaluation;
}

export interface FileEntry {
  name: string;
  path: string;
  isDir: boolean;
  size: number;
  modified?: string | null;
}

export interface FileProperties {
  name: string;
  path: string;
  remote: boolean;
  kind: string;
  isDir: boolean;
  isFile: boolean;
  isSymlink: boolean;
  size: number;
  permissions?: number | null;
  modified?: string | null;
  accessed?: string | null;
  created?: string | null;
}

export interface ExternalDropResult {
  tasks: TransferTask[];
  directoriesPrepared: number;
  skipped: string[];
  totalBytes: number;
}

export interface TmuxSessionInfo {
  name: string;
  windows: number;
  attached: number;
  created?: string | null;
}

export interface TmuxPaneInfo {
  session: string;
  windowIndex: number;
  paneIndex: number;
  paneId: string;
  active: boolean;
  command: string;
  title: string;
}

export interface TmuxState {
  sessions: TmuxSessionInfo[];
  panes: TmuxPaneInfo[];
}
