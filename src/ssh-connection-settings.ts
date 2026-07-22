import type { AuthMethod, SshConnection } from "./types";

export const sshConnectionDefaults = {
  reconnectDelayMs: 1_000,
  keepaliveEnabled: true,
  keepaliveIntervalSeconds: 30,
  keepaliveMaxMissed: 3,
} as const;

export const sshConnectionBounds = {
  reconnectDelayMs: { min: 100, max: 60_000 },
  keepaliveIntervalSeconds: { min: 1, max: 3_600 },
  keepaliveMaxMissed: { min: 1, max: 20 },
} as const;

const defaultAuthOrder: AuthMethod[] = ["public-key", "keyboard-interactive", "password"];

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
