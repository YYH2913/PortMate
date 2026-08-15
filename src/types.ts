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

export interface DeleteSessionProfileResponse {
  deletedProfileId: string;
  sessions: SessionSummary[];
  oneKeys: OneKeySummary[];
  hostKeys: HostKeyStore;
  grants: McpGrant[];
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
    backgroundOpacity: number;
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

export type ProxyKind = "http-connect" | "socks5";

export interface ProxyConfig {
  enabled: boolean;
  kind: ProxyKind;
  host: string;
  port: number;
  username: string;
  passwordSecretRef?: string | null;
}

export interface SshConnection {
  endpoint: { host: string; port: number };
  username: string;
  reconnect: boolean;
  reconnectDelayMs: number;
  keepaliveEnabled: boolean;
  keepaliveIntervalSeconds: number;
  keepaliveMaxMissed: number;
  tcpKeepaliveEnabled: boolean | null;
  proxy: ProxyConfig;
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
  reconnectDelayMs: number;
  receiveIdleTimeoutEnabled: boolean;
  receiveIdleTimeoutSeconds: number;
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
  proxy: ProxyConfig;
  reconnectDelayMs: number;
  keepaliveEnabled: boolean;
  keepaliveIdleSeconds: number;
  keepaliveIntervalSeconds: number;
  keepaliveRetries: number;
  telnetBinary: boolean;
  telnetNaws: boolean;
  tlsEnabled: boolean;
  tlsServerName: string | null;
  tlsAcceptInvalidCert: boolean;
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

export type OneKeyKind = "account" | "ssh";

export interface OneKeyIdentitySummary {
  sourceProfileId: string;
  id: string;
  label: string;
  source: IdentityRef["source"];
  fingerprintSha256?: string | null;
}

export interface OneKeySummary {
  id: string;
  label: string;
  kind: OneKeyKind;
  username: string;
  hasPassword: boolean;
  hasPassphrase: boolean;
  identity?: OneKeyIdentitySummary | null;
  sessionIds: string[];
  createdAt: string;
  updatedAt: string;
}

export type OneKeySecretUpdate =
  | { action: "preserve" }
  | { action: "clear" }
  | { action: "set"; secret: string; storage: "portable" };

export type OneKeyIdentityUpdate =
  | { action: "preserve" }
  | { action: "clear" }
  | { action: "set"; sourceProfileId: string; identityId: string };

export interface SaveOneKeyRequest {
  id?: string | null;
  label: string;
  kind: OneKeyKind;
  username: string;
  passwordUpdate: OneKeySecretUpdate;
  passphraseUpdate: OneKeySecretUpdate;
  identityUpdate: OneKeyIdentityUpdate;
  sessionIds: string[];
}

export interface OneKeyMutationResponse {
  items: OneKeySummary[];
  savedId: string;
}

export interface TunnelSpec {
  id: string;
  label: string;
  mode: "local" | "remote" | "dynamic";
  bindHost: string;
  bindPort: number;
  targetHost: string;
  targetPort: number;
  routeRules: TunnelRouteRule[];
  enabled: boolean;
}

export interface TunnelRouteRule {
  host: string;
  port: number | null;
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
  matcher: TriggerMatcher;
  actions: TriggerAction[];
  enabled: boolean;
}

export type TriggerMatcher =
  | { type: "contains"; text: string; case_sensitive: boolean }
  | { type: "regex"; pattern: string };

export type TriggerAction =
  | { type: "highlight"; color: string }
  | { type: "send-text"; text: string }
  | { type: "local-command"; command: string }
  | { type: "notification"; message: string }
  | { type: "timeline-mark"; label: string }
  | { type: "custom-link"; url_template: string }
  | { type: "sound"; name: string };

export interface TriggerEffect {
  sessionId: string;
  triggerId: string;
  triggerLabel: string;
  kind: "highlight" | "notification" | "custom-link" | "sound";
  value: string;
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

export interface TerminalBytesEvent {
  id: string;
  sessionId: string;
  ts: string;
  direction: "inbound" | "outbound";
  stream: "stdout" | "stderr" | "control" | "audit";
  bytes: number[];
  originalLength: number;
  truncated: boolean;
  eventId?: string | null;
}

export interface SerialCaptureFrame {
  id: string;
  ts: string;
  direction: "inbound" | "outbound";
  bytes: number[];
  originalLength: number;
  truncated: boolean;
}

export interface SerialCaptureSnapshot {
  frames: SerialCaptureFrame[];
  reset: boolean;
  totalFrames: number;
  capturedBytes: number;
}

export interface SerialCaptureHistorySnapshot {
  frames: SerialCaptureFrame[];
  enabled: boolean;
  totalFrames: number;
  capturedBytes: number;
  droppedFrames: number;
  unavailableFrames: number;
}

export interface ExportSerialCaptureResult {
  path: string;
  checksumPath: string;
  sha256: string;
  size: number;
  frames: number;
  capturedBytes: number;
  truncatedFrames: number;
}

export interface ExportTerminalTextResult {
  path: string;
  checksumPath: string;
  sha256: string;
  size: number;
  sessionId: string;
  viewId: string;
  source: "buffer" | "selection";
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
  signaturePath: string;
  sha256: string;
  signatureAlgorithm: string;
  signingPublicKey: string;
  size: number;
  files: number;
  rawLogSegments: number;
  attachments: number;
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

export interface CommandHistorySnapshot {
  entries: Array<{ command: string; recordedAt: number }>;
  migrated: boolean;
  revision: number;
}

export interface SysmonSnapshot {
  sessionId: string;
  ts: string;
  uptimeSeconds: number;
  cpuPercent: number;
  memoryPercent: number;
  rxKbps: number;
  txKbps: number;
  loadAverage: [number, number, number];
  memoryTotalBytes: number;
  memoryAvailableBytes: number;
  processes: SysmonProcess[];
  disks: SysmonDisk[];
  networkInterfaces: SysmonNetworkInterface[];
}

export interface SysmonProcess {
  pid: number;
  name: string;
  cpuPercent: number;
  memoryPercent: number;
  rssBytes: number;
}

export interface SysmonDisk {
  filesystem: string;
  mountPoint: string;
  totalBytes: number;
  availableBytes: number;
  usedPercent: number;
}

export interface SysmonNetworkInterface {
  name: string;
  addresses: string[];
  rxBytes: number;
  txBytes: number;
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

export interface ExportMcpAuditResult {
  path: string;
  checksumPath: string;
  sha256: string;
  size: number;
  records: number;
}

export type McpScope = "read-sessions" | "read-logs" | "read-transfers" | "read-tunnels" | "read-scripts" | "write-input" | "transfer" | "tunnel" | "manage-sessions" | "run-scripts";

export interface CustomScript {
  id: string;
  name: string;
  description: string;
  content: string;
  allowAllSessions: boolean;
  allowedSessionIds: string[];
  mcpEnabled: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface SaveCustomScriptRequest {
  id: string | null;
  name: string;
  description: string;
  content: string;
  allowAllSessions: boolean;
  allowedSessionIds: string[];
  mcpEnabled: boolean;
  expectedUpdatedAt: string | null;
}

export interface McpGrant {
  clientId: string;
  name: string;
  scopes: McpScope[];
  allowedSessions: string[];
  confirmWrites: boolean;
  expiresAt?: string | null;
  revokedAt?: string | null;
}

export interface McpApprovalRequest {
  id: string;
  clientId: string;
  action: string;
  sessionId: string;
  scope: McpScope;
  target?: McpApprovalTarget;
  createdAt: string;
  expiresAt: string;
}

export interface McpApprovalTarget {
  kind: "custom-script";
  id: string;
  label: string;
}

export interface McpHttpConfigRequest {
  listenHost: string;
  clientHost: string;
  port: number;
  allowedOrigins: string[];
  clientId: string;
  trusted: boolean;
  allowRemote: boolean;
}

export interface McpHttpConfig extends McpHttpConfigRequest {
  remoteAccess: boolean;
  endpoint: string;
  clientEndpoint: string;
  tokenRef: string;
  tokenAvailable: boolean;
  defaultOrigin: string;
  executable: string;
  storePath: string;
  startCommand: string;
}

export interface McpHttpTokenResponse {
  config: McpHttpConfig;
  token: string;
}

export type McpHttpRuntimePhase = "stopped" | "starting" | "running" | "failed";

export interface McpHttpRuntimeStatus {
  phase: McpHttpRuntimePhase;
  endpoint?: string | null;
  pid?: number | null;
  startedAt?: string | null;
  message?: string | null;
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

export interface SshHealthReport {
  sessionId: string;
  runtimeId: string;
  checkedAt: string;
  status: "healthy" | "degraded" | "unresponsive";
  backend: "russh" | "libssh";
  authenticationMethod: AuthMethod;
  terminalChannelOpen: boolean;
  transportRoundTripMs?: number | null;
  channelRoundTripMs?: number | null;
  sftpRoundTripMs?: number | null;
  transportError?: string | null;
  terminalError?: string | null;
  channelError?: string | null;
  sftpError?: string | null;
  sftpProbed: boolean;
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
  synchronized: boolean;
  command: string;
  title: string;
}

export interface TmuxWindowInfo {
  session: string;
  windowIndex: number;
  windowId: string;
  name: string;
  panes: number;
  active: boolean;
  synchronized: boolean;
}

export interface TmuxState {
  sessions: TmuxSessionInfo[];
  windows: TmuxWindowInfo[];
  panes: TmuxPaneInfo[];
}

export interface TmuxControlStatus {
  sessionId: string;
  target: string;
  active: boolean;
  runtimeId?: string | null;
}

export interface TmuxControlEvent extends TmuxControlStatus {
  kind: "started" | "state-changed" | "stopped";
  runtimeId: string;
  protocolEvent?: string | null;
  error?: string | null;
}

export type TmuxMutationAction =
  | "rename-session"
  | "kill-session"
  | "new-window"
  | "rename-window"
  | "kill-window"
  | "kill-pane"
  | "select-pane"
  | "break-pane"
  | "move-pane-horizontal"
  | "move-pane-vertical"
  | "split-pane-horizontal"
  | "split-pane-vertical"
  | "swap-pane-previous"
  | "swap-pane-next"
  | "resize-pane-left"
  | "resize-pane-right"
  | "resize-pane-up"
  | "resize-pane-down"
  | "select-layout";

export type TmuxWindowLayout = "even-horizontal" | "even-vertical" | "main-horizontal" | "main-vertical" | "tiled";

export interface TmuxMutationRequest {
  sessionId: string;
  action: TmuxMutationAction;
  target: string;
  name?: string | null;
  destination?: string | null;
  layout?: TmuxWindowLayout | null;
  amount?: number | null;
}
