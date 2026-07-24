import type { SessionSummary } from "./types";
import { normalizeSessionDisconnectReason } from "./session-runtime-state";
import {
  MAX_TRIGGER_ACTIONS,
  MAX_TRIGGER_ACTION_VALUE_CHARACTERS,
  MAX_TRIGGER_ID_CHARACTERS,
  MAX_TRIGGER_LABEL_CHARACTERS,
  MAX_TRIGGER_MATCHER_CHARACTERS,
  MAX_TRIGGERS_PER_PROFILE,
} from "./trigger-state";
import {
  MAX_TUNNELS_PER_PROFILE,
  MAX_TUNNEL_HOST_CHARACTERS,
  MAX_TUNNEL_ID_CHARACTERS,
  MAX_TUNNEL_LABEL_CHARACTERS,
} from "./tunnel-state";

export const SESSION_SUMMARY_CACHE_STORAGE_KEY = "portmate.sessions";

const SESSION_SUMMARY_CACHE_VERSION = 1;
const sessionKinds = ["ssh", "serial", "shell", "telnet", "tcp", "tmux"] as const;
const sessionStatuses = ["disconnected", "connecting", "connected", "reconnecting", "blocked", "error"] as const;
const authMethods = ["public-key", "keyboard-interactive", "password", "gssapi-with-mic", "none"] as const;

type CacheStorageReader = Pick<Storage, "getItem">;

export function readSessionSummaryCache(storage: CacheStorageReader): SessionSummary[] {
  try {
    return parseSessionSummaryCache(storage.getItem(SESSION_SUMMARY_CACHE_STORAGE_KEY));
  } catch {
    return [];
  }
}

export function parseSessionSummaryCache(raw: string | null): SessionSummary[] {
  if (!raw) return [];
  try {
    const decoded: unknown = JSON.parse(raw);
    const root = recordValue(decoded);
    const sessions = Array.isArray(decoded)
      ? decoded
      : root?.version === SESSION_SUMMARY_CACHE_VERSION && Array.isArray(root.sessions)
        ? root.sessions
        : null;
    if (!sessions || !sessions.every(isSessionSummary)) return [];
    const ids = sessions.map((session) => session.profile.id);
    if (new Set(ids).size !== ids.length) return [];
    return sessions.map(normalizeCachedSessionSummary);
  } catch {
    return [];
  }
}

function normalizeCachedSessionSummary(session: SessionSummary): SessionSummary {
  const lastDisconnectReason = normalizeSessionDisconnectReason(session.runtime.lastDisconnectReason);
  const connection = session.profile.connection;
  const profile = (connection.kind === "ssh" || connection.kind === "tmux")
    && (connection as { tcpKeepaliveEnabled?: unknown }).tcpKeepaliveEnabled === undefined
    ? {
        ...session.profile,
        connection: { ...connection, tcpKeepaliveEnabled: null },
      }
    : session.profile;
  return {
    ...session,
    profile,
    runtime: {
      ...session.runtime,
      lastDisconnectReason: lastDisconnectReason || null,
    },
  };
}

function isSessionSummary(value: unknown): value is SessionSummary {
  const summary = recordValue(value);
  const profile = recordValue(summary?.profile);
  const runtime = recordValue(summary?.runtime);
  if (!summary || !profile || !runtime || !isProfile(profile) || !isRuntime(runtime)) return false;
  return runtime.sessionId === profile.id
    && runtime.activeTransport === profile.kind
    && isNonNegativeInteger(summary.logLines)
    && isOptionalString(summary.lastLine);
}

function isProfile(profile: Record<string, unknown>): boolean {
  const kind = profile.kind;
  return isBoundedId(profile.id)
    && isString(profile.name)
    && includes(sessionKinds, kind)
    && isString(profile.group)
    && isStringArray(profile.tags)
    && isConnection(profile.connection, kind)
    && isTerminalProfile(profile.terminal)
    && isLoggingProfile(profile.logging)
    && isTriggerList(profile.triggers)
    && isTransferProfile(profile.transfer);
}

function isRuntime(runtime: Record<string, unknown>): boolean {
  return isBoundedId(runtime.sessionId)
    && isString(runtime.paneId)
    && includes(sessionStatuses, runtime.status)
    && isString(runtime.title)
    && isOptionalString(runtime.cwd)
    && isOptionalString(runtime.connectedSince)
    && isString(runtime.lastActivity)
    && isOptionalString(runtime.lastDisconnect)
    && isOptionalString(runtime.lastDisconnectReason)
    && includes(sessionKinds, runtime.activeTransport);
}

function isConnection(value: unknown, expectedKind: unknown): boolean {
  const connection = recordValue(value);
  if (!connection || connection.kind !== expectedKind) return false;
  switch (connection.kind) {
    case "shell":
      return isString(connection.program)
        && isStringArray(connection.args)
        && isOptionalString(connection.cwd);
    case "serial":
      return isString(connection.port)
        && isFiniteNumber(connection.baudRate)
        && isFiniteNumber(connection.dataBits)
        && isFiniteNumber(connection.stopBits)
        && isString(connection.parity)
        && isString(connection.flowControl)
        && isBoolean(connection.dtr)
        && isBoolean(connection.rts)
        && isBoolean(connection.reconnect)
        && isFiniteNumber(connection.reconnectDelayMs)
        && isBoolean(connection.receiveIdleTimeoutEnabled)
        && isFiniteNumber(connection.receiveIdleTimeoutSeconds);
    case "tcp":
    case "telnet":
      return isString(connection.host)
        && isFiniteNumber(connection.port)
        && isBoolean(connection.reconnect)
        && isProxy(connection.proxy)
        && isFiniteNumber(connection.reconnectDelayMs)
        && isBoolean(connection.keepaliveEnabled)
        && isFiniteNumber(connection.keepaliveIdleSeconds)
        && isFiniteNumber(connection.keepaliveIntervalSeconds)
        && isFiniteNumber(connection.keepaliveRetries)
        && isBoolean(connection.telnetBinary)
        && isBoolean(connection.telnetNaws);
    case "ssh":
    case "tmux": {
      const endpoint = recordValue(connection.endpoint);
      const identityPolicy = recordValue(connection.identityPolicy);
      const agentPolicy = recordValue(connection.agentPolicy);
      return endpoint !== null
        && isString(endpoint.host)
        && isFiniteNumber(endpoint.port)
        && isString(connection.username)
        && isBoolean(connection.reconnect)
        && isFiniteNumber(connection.reconnectDelayMs)
        && isBoolean(connection.keepaliveEnabled)
        && isFiniteNumber(connection.keepaliveIntervalSeconds)
        && isFiniteNumber(connection.keepaliveMaxMissed)
        && isOptionalNullableBoolean(connection.tcpKeepaliveEnabled)
        && isProxy(connection.proxy)
        && isOptionalString(connection.passwordSecretRef)
        && isOptionalString(connection.passphraseSecretRef)
        && isHostKeyPolicy(connection.hostKeyPolicy)
        && Array.isArray(connection.trustedHostKeys)
        && connection.trustedHostKeys.every(isTrustedHostKey)
        && identityPolicy !== null
        && isBoolean(identityPolicy.identitiesOnly)
        && Array.isArray(identityPolicy.authOrder)
        && identityPolicy.authOrder.every((method) => includes(authMethods, method))
        && isBoolean(identityPolicy.recordSuccess)
        && (identityPolicy.lastSuccessful === null
          || identityPolicy.lastSuccessful === undefined
          || includes(authMethods, identityPolicy.lastSuccessful))
        && Array.isArray(connection.identityRefs)
        && connection.identityRefs.every(isIdentityRef)
        && agentPolicy !== null
        && isBoolean(agentPolicy.enabled)
        && isBoolean(agentPolicy.forwarding)
        && includes(["disabled", "after-profile-keys", "before-profile-keys"] as const, agentPolicy.offerMode)
        && Array.isArray(connection.jumps)
        && connection.jumps.every(isJumpHop)
        && isTunnelList(connection.tunnels);
    }
    default:
      return false;
  }
}

function isTerminalProfile(value: unknown): boolean {
  const terminal = recordValue(value);
  return terminal !== null
    && isString(terminal.term)
    && isFiniteNumber(terminal.rows)
    && isFiniteNumber(terminal.cols)
    && isFiniteNumber(terminal.scrollback)
    && isString(terminal.fontFamily)
    && isFiniteNumber(terminal.fontSize)
    && isString(terminal.theme)
    && isFiniteNumber(terminal.backgroundOpacity);
}

function isLoggingProfile(value: unknown): boolean {
  const logging = recordValue(value);
  return logging !== null
    && isBoolean(logging.enabled)
    && isBoolean(logging.raw)
    && isBoolean(logging.text)
    && isBoolean(logging.jsonl)
    && isBoolean(logging.redactSecrets)
    && isString(logging.pathTemplate)
    && isFiniteNumber(logging.retentionDays);
}

function isTransferProfile(value: unknown): boolean {
  const transfer = recordValue(value);
  return transfer !== null
    && isBoolean(transfer.sftp)
    && isBoolean(transfer.scp)
    && isBoolean(transfer.xmodem)
    && isBoolean(transfer.ymodem)
    && isBoolean(transfer.zmodem)
    && isOptionalNumber(transfer.rateLimitBytesPerSecond)
    && isOptionalString(transfer.defaultLocalDir);
}

function isProxy(value: unknown): boolean {
  const proxy = recordValue(value);
  return proxy !== null
    && isBoolean(proxy.enabled)
    && includes(["http-connect", "socks5"] as const, proxy.kind)
    && isString(proxy.host)
    && isFiniteNumber(proxy.port)
    && isString(proxy.username)
    && isOptionalString(proxy.passwordSecretRef);
}

function isHostKeyPolicy(value: unknown): boolean {
  const policy = recordValue(value);
  return policy !== null
    && includes(["strict", "trust-on-first-use", "ask-every-time"] as const, policy.mode)
    && isOptionalString(policy.alias)
    && includes(["profile", "project", "user"] as const, policy.trustScope)
    && isBoolean(policy.allowRotation)
    && isBoolean(policy.checkIp);
}

function isTrustedHostKey(value: unknown): boolean {
  const key = recordValue(value);
  return key !== null
    && isString(key.id)
    && isOptionalString(key.profileId)
    && isString(key.alias)
    && isString(key.host)
    && isFiniteNumber(key.port)
    && isString(key.algorithm)
    && isString(key.fingerprintSha256)
    && isString(key.publicKeyBase64)
    && includes(["profile", "project", "user"] as const, key.scope)
    && isOptionalString(key.label)
    && isString(key.firstSeen)
    && isString(key.lastSeen);
}

function isIdentityRef(value: unknown): boolean {
  const identity = recordValue(value);
  return identity !== null
    && isString(identity.id)
    && isString(identity.label)
    && includes(["profile-vault", "system-file", "agent", "public-key-only"] as const, identity.source)
    && isOptionalString(identity.fingerprintSha256)
    && isOptionalString(identity.path)
    && isOptionalString(identity.secretRef);
}

function isJumpHop(value: unknown): boolean {
  const jump = recordValue(value);
  return jump !== null
    && isString(jump.host)
    && isFiniteNumber(jump.port)
    && isString(jump.username)
    && isOptionalString(jump.passwordSecretRef)
    && isOptionalString(jump.passphraseSecretRef)
    && isOptionalString(jump.identityRef)
    && (jump.hostKeyPolicy === null
      || jump.hostKeyPolicy === undefined
      || isHostKeyPolicy(jump.hostKeyPolicy));
}

function isTunnelSpec(value: unknown): boolean {
  const tunnel = recordValue(value);
  if (tunnel === null
    || !isBoundedTunnelText(tunnel.id, MAX_TUNNEL_ID_CHARACTERS, false, false)
    || !isBoundedTunnelText(tunnel.label, MAX_TUNNEL_LABEL_CHARACTERS, false, false)
    || !includes(["local", "remote", "dynamic"] as const, tunnel.mode)
    || !isTunnelPort(tunnel.bindPort, true)
    || !isBoolean(tunnel.enabled)) return false;
  const mode = tunnel.mode as "local" | "remote" | "dynamic";
  if (!isBoundedTunnelText(
    tunnel.bindHost,
    MAX_TUNNEL_HOST_CHARACTERS,
    mode === "remote",
    true,
  )) return false;
  return mode === "dynamic"
    ? tunnel.targetHost === "" && tunnel.targetPort === 0
    : isBoundedTunnelText(
      tunnel.targetHost,
      MAX_TUNNEL_HOST_CHARACTERS,
      false,
      true,
    ) && isTunnelPort(tunnel.targetPort, false);
}

function isTunnelList(value: unknown): boolean {
  if (!Array.isArray(value) || value.length > MAX_TUNNELS_PER_PROFILE) return false;
  const ids = new Set<string>();
  return value.every((tunnel) => {
    const record = recordValue(tunnel);
    if (!isTunnelSpec(record) || ids.has(record?.id as string)) return false;
    ids.add(record?.id as string);
    return true;
  });
}

function isTunnelPort(value: unknown, allowZero: boolean): boolean {
  return isFiniteNumber(value)
    && Number.isInteger(value)
    && value >= (allowZero ? 0 : 1)
    && value <= 65_535;
}

function isBoundedTunnelText(
  value: unknown,
  maxCharacters: number,
  allowEmpty: boolean,
  rejectWhitespace: boolean,
): value is string {
  if (typeof value !== "string" || value.trim() !== value || (!allowEmpty && !value)) return false;
  let count = 0;
  for (const character of value) {
    count += 1;
    if (count > maxCharacters || (rejectWhitespace && /\s/u.test(character))) return false;
    const codePoint = character.codePointAt(0) ?? 0;
    if (codePoint <= 0x1f || (codePoint >= 0x7f && codePoint <= 0x9f)) return false;
  }
  return true;
}

function isTriggerList(value: unknown): boolean {
  if (!Array.isArray(value) || value.length > MAX_TRIGGERS_PER_PROFILE) return false;
  const ids = new Set<string>();
  return value.every((trigger) => {
    const record = recordValue(trigger);
    if (!isTrigger(record) || ids.has(record.id as string)) return false;
    ids.add(record.id as string);
    return true;
  });
}

function isTrigger(
  trigger: Record<string, unknown> | null,
): trigger is Record<string, unknown> & { id: string } {
  const matcher = recordValue(trigger?.matcher);
  return trigger !== null
    && isBoundedTriggerText(trigger.id, MAX_TRIGGER_ID_CHARACTERS, false)
    && (trigger.id as string).trim() === trigger.id
    && !/[\u0000-\u001f\u007f-\u009f]/.test(trigger.id as string)
    && isBoundedTriggerText(trigger.label, MAX_TRIGGER_LABEL_CHARACTERS, true)
    && !/[\u0000-\u001f\u007f-\u009f]/.test(trigger.label as string)
    && matcher !== null
    && (matcher.type === "contains"
      ? isBoundedTriggerText(matcher.text, MAX_TRIGGER_MATCHER_CHARACTERS, false) && isBoolean(matcher.case_sensitive)
      : matcher.type === "regex" && isBoundedTriggerText(matcher.pattern, MAX_TRIGGER_MATCHER_CHARACTERS, false))
    && Array.isArray(trigger.actions)
    && trigger.actions.length <= MAX_TRIGGER_ACTIONS
    && trigger.actions.every(isTriggerAction)
    && isBoolean(trigger.enabled);
}

function isTriggerAction(value: unknown): boolean {
  const action = recordValue(value);
  if (!action) return false;
  switch (action.type) {
    case "highlight":
      return typeof action.color === "string" && /^#[0-9a-f]{6}$/i.test(action.color);
    case "send-text":
      return isBoundedTriggerText(action.text, MAX_TRIGGER_ACTION_VALUE_CHARACTERS, true);
    case "local-command":
      return isBoundedTriggerText(action.command, MAX_TRIGGER_ACTION_VALUE_CHARACTERS, false)
        && (action.command as string).trim().length > 0;
    case "notification":
      return isBoundedTriggerText(action.message, MAX_TRIGGER_ACTION_VALUE_CHARACTERS, true);
    case "timeline-mark":
      return isBoundedTriggerText(action.label, MAX_TRIGGER_ACTION_VALUE_CHARACTERS, true);
    case "custom-link":
      return isBoundedTriggerText(action.url_template, MAX_TRIGGER_ACTION_VALUE_CHARACTERS, true);
    case "sound":
      return includes(["bell", "chime", "alert"] as const, action.name);
    default:
      return false;
  }
}

function isBoundedTriggerText(value: unknown, maxCharacters: number, allowEmpty: boolean): value is string {
  return typeof value === "string"
    && (allowEmpty || value.length > 0)
    && value.length <= maxCharacters
    && !value.includes("\0");
}

function recordValue(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function includes<const T extends readonly unknown[]>(values: T, value: unknown): value is T[number] {
  return (values as readonly unknown[]).includes(value);
}

function isString(value: unknown): value is string {
  return typeof value === "string";
}

function isBoundedId(value: unknown): value is string {
  return typeof value === "string"
    && value.length > 0
    && value.length <= 256
    && !/[\u0000-\u001f\u007f]/.test(value);
}

function isOptionalString(value: unknown): value is string | null | undefined {
  return value === null || value === undefined || isString(value);
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every(isString);
}

function isBoolean(value: unknown): value is boolean {
  return typeof value === "boolean";
}

function isNullableBoolean(value: unknown): value is boolean | null {
  return value === null || isBoolean(value);
}

function isOptionalNullableBoolean(value: unknown): value is boolean | null | undefined {
  return value === undefined || isNullableBoolean(value);
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isOptionalNumber(value: unknown): value is number | null | undefined {
  return value === null || value === undefined || isFiniteNumber(value);
}

function isNonNegativeInteger(value: unknown): value is number {
  return isFiniteNumber(value) && Number.isInteger(value) && value >= 0;
}
