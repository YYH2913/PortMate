export interface TerminalTimestampEntry {
  line: number;
  ts: string;
}

export interface VisibleTerminalTimestamp extends TerminalTimestampEntry {
  row: number;
}

export const MAX_TERMINAL_TIMESTAMPS = 10_000_000;
const RFC3339_TIMESTAMP = /^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})(?:\.(\d{1,9}))?(Z|[+-]\d{2}:\d{2})$/i;

export function normalizeTerminalTimestamps(
  value: unknown,
  limit = MAX_TERMINAL_TIMESTAMPS,
): TerminalTimestampEntry[] {
  if (!Array.isArray(value)) return [];
  const byLine = new Map<number, TerminalTimestampEntry>();
  for (const candidate of value) {
    if (!candidate || typeof candidate !== "object" || Array.isArray(candidate)) continue;
    const entry = candidate as Record<string, unknown>;
    const line = normalizeTerminalTimestampLine(entry.line);
    const ts = normalizeTerminalTimestampValue(entry.ts);
    if (line === null || !ts || byLine.has(line)) continue;
    byLine.set(line, { line, ts });
  }
  const normalized = [...byLine.values()].sort((left, right) => left.line - right.line);
  return normalized.slice(-Math.max(1, Math.trunc(limit) || 1));
}

export function visibleTerminalTimestamps(
  entries: readonly TerminalTimestampEntry[],
  viewportY: number,
  rows: number,
): VisibleTerminalTimestamp[] {
  const firstLine = Math.max(0, Math.trunc(viewportY) || 0);
  const rowCount = Math.max(0, Math.trunc(rows) || 0);
  if (!rowCount) return [];
  const normalized = normalizeTerminalTimestamps(entries, Math.max(1, entries.length));
  if (!normalized.length) return [];

  let timestampIndex = -1;
  while (timestampIndex + 1 < normalized.length
    && normalized[timestampIndex + 1].line <= firstLine) timestampIndex += 1;

  const visible: VisibleTerminalTimestamp[] = [];
  for (let row = 0; row < rowCount; row += 1) {
    const line = firstLine + row;
    while (timestampIndex + 1 < normalized.length
      && normalized[timestampIndex + 1].line <= line) timestampIndex += 1;
    if (timestampIndex >= 0) visible.push({ line, row, ts: normalized[timestampIndex].ts });
  }
  return visible;
}

export function rebaseTerminalTimestamps(
  entries: readonly TerminalTimestampEntry[],
  firstLine: number,
  limit = MAX_TERMINAL_TIMESTAMPS,
): TerminalTimestampEntry[] {
  const offset = Math.max(0, Math.trunc(firstLine) || 0);
  const normalized = normalizeTerminalTimestamps(entries, Math.max(1, entries.length));
  if (!normalized.length) return [];
  const rebased: TerminalTimestampEntry[] = [];
  let preceding: TerminalTimestampEntry | null = null;
  for (const entry of normalized) {
    if (entry.line <= offset) {
      preceding = entry;
      continue;
    }
    rebased.push({ line: entry.line - offset, ts: entry.ts });
  }
  if (preceding) rebased.unshift({ line: 0, ts: preceding.ts });
  return normalizeTerminalTimestamps(rebased, limit);
}

export function formatTerminalTimestampClock(value: string): string {
  const normalized = normalizeTerminalTimestampValue(value);
  if (!normalized) return "--:--:--.------";
  const date = new Date(normalized);
  const fraction = normalized.match(/\.(\d+)Z$/)?.[1].slice(0, 6).padEnd(6, "0") ?? "000000";
  return `${padClock(date.getHours())}:${padClock(date.getMinutes())}:${padClock(date.getSeconds())}.${fraction}`;
}

function normalizeTerminalTimestampLine(value: unknown): number | null {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) return null;
  return value;
}

function normalizeTerminalTimestampValue(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const parsed = Date.parse(value);
  if (!Number.isFinite(parsed)) return null;
  const match = value.match(RFC3339_TIMESTAMP);
  const fraction = match?.[2]
    ? match[2].slice(0, 6).padEnd(6, "0")
    : new Date(parsed).toISOString().match(/\.(\d{3})Z$/)?.[1].padEnd(6, "0") ?? "000000";
  const utcSecond = new Date(Math.floor(parsed / 1_000) * 1_000).toISOString().slice(0, 19);
  return `${utcSecond}.${fraction}Z`;
}

function padClock(value: number): string {
  return String(value).padStart(2, "0");
}
