export interface TerminalTimestampEntry {
  line: number;
  ts: string;
}

export interface VisibleTerminalTimestamp extends TerminalTimestampEntry {
  row: number;
}

export const MAX_TERMINAL_TIMESTAMPS = 4_000;

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
  const lastLine = firstLine + rowCount;
  const visible: VisibleTerminalTimestamp[] = [];
  const seenLines = new Set<number>();
  for (const entry of entries) {
    const line = normalizeTerminalTimestampLine(entry.line);
    const ts = normalizeTerminalTimestampValue(entry.ts);
    if (line === null || !ts || line < firstLine || line >= lastLine || seenLines.has(line)) continue;
    seenLines.add(line);
    visible.push({ line, row: line - firstLine, ts });
  }
  return visible.sort((left, right) => left.line - right.line);
}

function normalizeTerminalTimestampLine(value: unknown): number | null {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) return null;
  return value;
}

function normalizeTerminalTimestampValue(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? new Date(parsed).toISOString() : null;
}
