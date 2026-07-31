export const COMMAND_HISTORY_STORAGE_KEY = "portmate.commandHistory";
export const COMMAND_HISTORY_VERSION = 2;
export const DEFAULT_COMMAND_HISTORY_LIMIT = 10_000;
export const MAX_COMMAND_HISTORY_LIMIT = 10_000;
export const MAX_COMMAND_HISTORY_RETENTION_DAYS = 3_650;
export const MAX_COMMAND_HISTORY_COMMAND_CHARACTERS = 8_192;
export const MAX_COMMAND_HISTORY_STORAGE_BYTES = 2 * 1024 * 1024;

export type CommandHistoryEntry = {
  command: string;
  recordedAt: number;
};

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
    const command = typeof item === "string"
      ? normalizeCommandHistoryCommand(item)
      : item && typeof item === "object" && !Array.isArray(item)
        ? normalizeCommandHistoryCommand((item as Record<string, unknown>).command)
        : null;
    if (!command || seen.has(command)) continue;
    const rawTimestamp = typeof item === "string"
      ? now - index
      : Number((item as Record<string, unknown>).recordedAt);
    if (!Number.isFinite(rawTimestamp)) continue;
    const recordedAt = Math.min(now, Math.max(0, Math.trunc(rawTimestamp)));
    if (recordedAt < cutoff) continue;
    const entryBytes = utf8Bytes(JSON.stringify({ command, recordedAt })) + (entries.length ? 1 : 0);
    if (bytes + entryBytes > MAX_COMMAND_HISTORY_STORAGE_BYTES) continue;
    seen.add(command);
    entries.push({ command, recordedAt });
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
): CommandHistoryEntry[] {
  const valid = normalizeCommandHistoryCommand(command);
  const withoutDuplicate = valid
    ? current.filter((entry) => entry.command !== valid)
    : current;
  return normalizeCommandHistory({
    version: COMMAND_HISTORY_VERSION,
    entries: valid ? [{ command: valid, recordedAt: now }, ...withoutDuplicate] : withoutDuplicate,
  }, policy, now);
}

export function queuePendingCommandHistory(
  current: readonly string[],
  command: string,
  policy: CommandHistoryPolicy,
  now = Date.now(),
): string[] {
  const valid = normalizeCommandHistoryCommand(command);
  if (!valid) return [...current];
  const candidates = [...current.filter((item) => item !== valid), valid]
    .slice(-MAX_COMMAND_HISTORY_LIMIT * 2)
    .reverse()
    .map((item, index) => ({ command: item, recordedAt: Math.max(0, now - index) }));
  return commandHistoryCommands(normalizeCommandHistory({
    version: COMMAND_HISTORY_VERSION,
    entries: candidates,
  }, policy, now)).reverse();
}

export function commandHistoryCommands(entries: readonly CommandHistoryEntry[]): string[] {
  return entries.map((entry) => entry.command);
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
      entry.command === right[index]?.command && entry.recordedAt === right[index]?.recordedAt
    ));
}

function commandHistorySource(value: unknown): unknown[] {
  if (Array.isArray(value)) return value;
  if (!value || typeof value !== "object") return [];
  const root = value as Record<string, unknown>;
  return root.version === COMMAND_HISTORY_VERSION && Array.isArray(root.entries)
    ? root.entries
    : [];
}

export function normalizeCommandHistoryCommand(value: unknown): string | null {
  if (typeof value !== "string" || !value.trim() || value.includes("\0")) return null;
  return Array.from(value).length <= MAX_COMMAND_HISTORY_COMMAND_CHARACTERS ? value : null;
}

function clampInteger(value: unknown, minimum: number, maximum: number, fallback: number): number {
  const parsed = typeof value === "number" ? value : typeof value === "string" ? Number(value) : Number.NaN;
  if (!Number.isFinite(parsed)) return fallback;
  return Math.min(maximum, Math.max(minimum, Math.trunc(parsed)));
}

function utf8Bytes(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}
