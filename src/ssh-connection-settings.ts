import type { AuthMethod, SshConnection } from "./types";

export const sshConnectionDefaults = {
  reconnectDelayMs: 1_000,
  reconnectIgnoreHostKeyChanges: false,
  keepaliveEnabled: true,
  keepaliveIntervalSeconds: 30,
  keepaliveMaxMissed: 3,
  tcpKeepaliveEnabled: null,
} as const;

export const sshConnectionBounds = {
  reconnectDelayMs: { min: 100, max: 60_000 },
  keepaliveIntervalSeconds: { min: 1, max: 3_600 },
  keepaliveMaxMissed: { min: 0, max: 20 },
} as const;

const defaultAuthOrder: AuthMethod[] = ["public-key", "keyboard-interactive", "password"];

export function sshUsesNoneAuthenticationOnly(connection: SshConnection): boolean {
  return connection.identityPolicy.authOrder.length > 0
    && connection.identityPolicy.authOrder.every(method => method === "none");
}

export const SSH_AUTH_METHODS: readonly AuthMethod[] = ["public-key", "keyboard-interactive", "password", "gssapi-with-mic", "none"];

export function moveSshAuthMethod(order: readonly AuthMethod[], method: AuthMethod, offset: -1 | 1): AuthMethod[] {
  const next = [...order];
  const index = next.indexOf(method);
  const target = index + offset;
  if (index < 0 || target < 0 || target >= next.length) return next;
  [next[index], next[target]] = [next[target], next[index]];
  return next;
}

export function toggleSshAuthMethod(order: readonly AuthMethod[], method: AuthMethod): AuthMethod[] {
  return order.includes(method)
    ? order.length > 1 ? order.filter(current => current !== method) : [...order]
    : [...order, method];
}

function boundedInteger(value: unknown, fallback: number, min: number, max: number): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  return Math.min(max, Math.max(min, Math.trunc(value)));
}

export function normalizeSshConnectionSettings<T extends SshConnection>(connection: T): T {
  const authOrder = normalizeAuthOrder(connection.identityPolicy.authOrder);
  const recordSuccess = typeof connection.identityPolicy.recordSuccess === "boolean"
    ? connection.identityPolicy.recordSuccess
    : true;
  const lastSuccessful = normalizeAuthMethod(connection.identityPolicy.lastSuccessful);
  return {
    ...connection,
    reconnect: typeof connection.reconnect === "boolean" ? connection.reconnect : true,
    reconnectIgnoreHostKeyChanges: connection.reconnectIgnoreHostKeyChanges === true,
    reconnectDelayMs: boundedInteger(
      connection.reconnectDelayMs,
      sshConnectionDefaults.reconnectDelayMs,
      sshConnectionBounds.reconnectDelayMs.min,
      sshConnectionBounds.reconnectDelayMs.max,
    ),
    keepaliveEnabled: typeof connection.keepaliveEnabled === "boolean" ? connection.keepaliveEnabled : true,
    keepaliveIntervalSeconds: boundedInteger(
      connection.keepaliveIntervalSeconds,
      sshConnectionDefaults.keepaliveIntervalSeconds,
      sshConnectionBounds.keepaliveIntervalSeconds.min,
      sshConnectionBounds.keepaliveIntervalSeconds.max,
    ),
    keepaliveMaxMissed: boundedInteger(
      connection.keepaliveMaxMissed,
      sshConnectionDefaults.keepaliveMaxMissed,
      sshConnectionBounds.keepaliveMaxMissed.min,
      sshConnectionBounds.keepaliveMaxMissed.max,
    ),
    tcpKeepaliveEnabled: typeof connection.tcpKeepaliveEnabled === "boolean"
      ? connection.tcpKeepaliveEnabled
      : null,
    identityPolicy: {
      ...connection.identityPolicy,
      identitiesOnly: typeof connection.identityPolicy.identitiesOnly === "boolean"
        ? connection.identityPolicy.identitiesOnly
        : true,
      authOrder,
      recordSuccess,
      lastSuccessful: recordSuccess && lastSuccessful && authOrder.includes(lastSuccessful)
        ? lastSuccessful
        : null,
    },
  } as T;
}

function normalizeAuthOrder(value: unknown): AuthMethod[] {
  const methods = Array.isArray(value)
    ? value.map(normalizeAuthMethod).filter((method): method is AuthMethod => method !== null)
    : [];
  const unique = methods.filter((method, index) => methods.indexOf(method) === index);
  return unique.length ? unique : [...defaultAuthOrder];
}

function normalizeAuthMethod(value: unknown): AuthMethod | null {
  if (value === "publickey") return "public-key";
  if (value === "gssapi") return "gssapi-with-mic";
  if (
    value === "public-key"
    || value === "keyboard-interactive"
    || value === "password"
    || value === "gssapi-with-mic"
    || value === "none"
  ) {
    return value;
  }
  return null;
}
