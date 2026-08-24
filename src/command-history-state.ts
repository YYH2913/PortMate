export const COMMAND_HISTORY_STORAGE_KEY = "portmate.commandHistory";
export const COMMAND_HISTORY_VERSION = 3;
export const DEFAULT_COMMAND_HISTORY_LIMIT = 10_000;
export const MAX_COMMAND_HISTORY_LIMIT = 10_000;
export const MAX_COMMAND_HISTORY_RETENTION_DAYS = 3_650;
export const MAX_COMMAND_HISTORY_COMMAND_CHARACTERS = 8_192;
export const MAX_COMMAND_HISTORY_STORAGE_BYTES = 2 * 1024 * 1024;
export const MAX_COMMAND_HISTORY_SESSION_ID_CHARACTERS = 256;

export type CommandHistoryEntry = {
  command: string;
  recordedAt: number;
  sessionId: string | null;
};

export type PendingCommandHistoryEntry = Pick<CommandHistoryEntry, "command" | "sessionId">;

export type CommandHistoryPolicy = {
  limit: number;
  retentionDays: number;
};

export type CommandHistorySnapshot = {
  version: typeof COMMAND_HISTORY_VERSION;
  entries: CommandHistoryEntry[];
};

export function normalizeCommandHistoryPolicy(
  limit: unknown,
  retentionDays: unknown,
): CommandHistoryPolicy {
  return {
    limit: clampInteger(limit, 1, MAX_COMMAND_HISTORY_LIMIT, DEFAULT_COMMAND_HISTORY_LIMIT),
    retentionDays: clampInteger(retentionDays, 0, MAX_COMMAND_HISTORY_RETENTION_DAYS, 30),
  };
}

export function normalizeCommandHistory(
  value: unknown,
  policy: CommandHistoryPolicy,
  now = Date.now(),
): CommandHistoryEntry[] {
  const source = commandHistorySource(value);
  const cutoff = policy.retentionDays
    ? now - policy.retentionDays * 24 * 60 * 60 * 1000
    : Number.NEGATIVE_INFINITY;
  const seen = new Set<string>();
  const entries: CommandHistoryEntry[] = [];
  let bytes = utf8Bytes(JSON.stringify({ version: COMMAND_HISTORY_VERSION, entries: [] }));

  for (let index = 0; index < Math.min(source.length, MAX_COMMAND_HISTORY_LIMIT * 2); index += 1) {
    const item = source[index];
    const record = item && typeof item === "object" && !Array.isArray(item)
      ? item as Record<string, unknown>
      : null;
    const command = typeof item === "string"
      ? normalizeCommandHistoryCommand(item)
      : normalizeCommandHistoryCommand(record?.command);
    const sessionId = typeof item === "string"
      ? null
      : normalizeCommandHistorySessionId(record?.sessionId);
    if (!command) continue;
    const identity = commandHistoryIdentity(command, sessionId);
    if (seen.has(identity)) continue;
    const rawTimestamp = typeof item === "string"
      ? now - index
      : Number(record?.recordedAt);
    if (!Number.isFinite(rawTimestamp)) continue;
    const recordedAt = Math.min(now, Math.max(0, Math.trunc(rawTimestamp)));
    if (recordedAt < cutoff) continue;
    const entryBytes = utf8Bytes(JSON.stringify({ command, recordedAt, sessionId })) + (entries.length ? 1 : 0);
    if (bytes + entryBytes > MAX_COMMAND_HISTORY_STORAGE_BYTES) continue;
    seen.add(identity);
    entries.push({ command, recordedAt, sessionId });
    bytes += entryBytes;
    if (entries.length >= policy.limit) break;
  }
  return entries;
}

export function recordCommandHistory(
  current: readonly CommandHistoryEntry[],
  command: string,
  policy: CommandHistoryPolicy,
  now = Date.now(),
  sessionId: string | null = null,
): CommandHistoryEntry[] {
  const valid = normalizeCommandHistoryCommand(command);
  const normalizedSessionId = normalizeCommandHistorySessionId(sessionId);
  const withoutDuplicate = valid
    ? current.filter((entry) => (
      entry.command !== valid || entry.sessionId !== normalizedSessionId
    ))
    : current;
  return normalizeCommandHistory({
    version: COMMAND_HISTORY_VERSION,
    entries: valid
      ? [{ command: valid, recordedAt: now, sessionId: normalizedSessionId }, ...withoutDuplicate]
      : withoutDuplicate,
  }, policy, now);
}

export function queuePendingCommandHistory(
  current: readonly PendingCommandHistoryEntry[],
  command: string,
  policy: CommandHistoryPolicy,
  now = Date.now(),
  sessionId: string | null = null,
): PendingCommandHistoryEntry[] {
  const valid = normalizeCommandHistoryCommand(command);
  if (!valid) return [...current];
  const normalizedSessionId = normalizeCommandHistorySessionId(sessionId);
  return normalizePendingCommandHistory(
    [
      ...current.filter((item) => (
        item.command !== valid || item.sessionId !== normalizedSessionId
      )),
      { command: valid, sessionId: normalizedSessionId },
    ],
    policy,
    now,
  );
}

export function normalizePendingCommandHistory(
  current: readonly (PendingCommandHistoryEntry | string)[],
  policy: CommandHistoryPolicy,
  now = Date.now(),
): PendingCommandHistoryEntry[] {
  const candidates = current
    .slice(-MAX_COMMAND_HISTORY_LIMIT * 2)
    .reverse()
    .map((item, index) => ({
      command: typeof item === "string" ? item : item.command,
      sessionId: typeof item === "string" ? null : item.sessionId,
      recordedAt: Math.max(0, now - index),
    }));
  return normalizeCommandHistory({
    version: COMMAND_HISTORY_VERSION,
    entries: candidates,
  }, policy, now).reverse().map(({ command, sessionId }) => ({ command, sessionId }));
}

export function commandHistoryEntriesForSession(
  entries: readonly CommandHistoryEntry[],
  sessionId: string | null | undefined,
): CommandHistoryEntry[] {
  if (!sessionId) return [...entries];
  return entries.filter((entry) => entry.sessionId === null || entry.sessionId === sessionId);
}

export function commandHistoryCommands(
  entries: readonly CommandHistoryEntry[],
  sessionId?: string | null,
): string[] {
  return commandHistoryEntriesForSession(entries, sessionId).map((entry) => entry.command);
}

export function commandHistorySnapshot(entries: readonly CommandHistoryEntry[]): CommandHistorySnapshot {
  return {
    version: COMMAND_HISTORY_VERSION,
    entries: entries.map((entry) => ({ ...entry })),
  };
}

export function commandHistoryEntriesEqual(
  left: readonly CommandHistoryEntry[],
  right: readonly CommandHistoryEntry[],
): boolean {
  return left.length === right.length
    && left.every((entry, index) => (
      entry.command === right[index]?.command
      && entry.recordedAt === right[index]?.recordedAt
      && entry.sessionId === right[index]?.sessionId
    ));
}

function commandHistorySource(value: unknown): unknown[] {
  if (Array.isArray(value)) return value;
  if (!value || typeof value !== "object") return [];
  const root = value as Record<string, unknown>;
  return (root.version === 2 || root.version === COMMAND_HISTORY_VERSION) && Array.isArray(root.entries)
    ? root.entries
    : [];
}

export function normalizeCommandHistoryCommand(value: unknown): string | null {
  if (typeof value !== "string" || !value.trim() || value.includes("\0")) return null;
  return Array.from(value).length <= MAX_COMMAND_HISTORY_COMMAND_CHARACTERS ? value : null;
}

export function normalizeCommandHistorySessionId(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const normalized = value.trim();
  return normalized
    && Array.from(normalized).length <= MAX_COMMAND_HISTORY_SESSION_ID_CHARACTERS
    && !/[\u0000-\u001f\u007f]/.test(normalized)
    ? normalized
    : null;
}

function commandHistoryIdentity(command: string, sessionId: string | null): string {
  return `${sessionId ?? ""}\u0000${command}`;
}

function clampInteger(value: unknown, minimum: number, maximum: number, fallback: number): number {
  const parsed = typeof value === "number" ? value : typeof value === "string" ? Number(value) : Number.NaN;
  if (!Number.isFinite(parsed)) return fallback;
  return Math.min(maximum, Math.max(minimum, Math.trunc(parsed)));
}

function utf8Bytes(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}
