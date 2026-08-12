import type { TerminalMouseEncoding } from "./terminal-mouse";
import { normalizeTerminalTimestamps, resizeAlternateTerminalTimestamps } from "./terminal-timestamp-state";
import type { TerminalTimestampEntry } from "./terminal-timestamp-state";

export type SerializedTerminalState = {
  serialized: string;
  cols: number;
  rows: number;
  seenEventIds: string[];
  mouseEncoding?: TerminalMouseEncoding;
  timestamps?: TerminalTimestampEntry[];
  alternateTimestamps?: TerminalTimestampEntry[];
  /** Legacy fallback retained for snapshots created before per-row alternate timestamps. */
  alternateTimestamp?: string;
};

export const MAX_SERIALIZED_TERMINALS = 32;
export const MAX_SERIALIZED_TERMINAL_BYTES = 2 * 1024 * 1024;
export const MAX_SERIALIZED_TERMINAL_EVENTS = 4000;

export function terminalStateCacheKey(sessionId: string, viewId: string): string {
  return JSON.stringify([sessionId, viewId]);
}

export function terminalEventSnapshotIds(
  seen: ReadonlySet<string>,
  polledEventIds: readonly string[],
  maxEvents = MAX_SERIALIZED_TERMINAL_EVENTS,
): string[] {
  const prioritized = new Set(seen);
  for (const eventId of polledEventIds) {
    if (!eventId) continue;
    prioritized.delete(eventId);
    prioritized.add(eventId);
  }
  return [...prioritized].slice(-Math.max(1, Math.trunc(maxEvents)));
}

export function rememberTerminalEventId(
  seen: Set<string>,
  pending: Set<string>,
  eventId: string,
  maxEvents = MAX_SERIALIZED_TERMINAL_EVENTS,
): boolean {
  if (seen.has(eventId)) return false;
  seen.add(eventId);
  pending.add(eventId);
  trimTerminalEventIds(seen, pending, maxEvents);
  return true;
}

export function settleTerminalEventId(
  seen: Set<string>,
  pending: Set<string>,
  eventId: string,
  maxEvents = MAX_SERIALIZED_TERMINAL_EVENTS,
) {
  pending.delete(eventId);
  trimTerminalEventIds(seen, pending, maxEvents);
}

function trimTerminalEventIds(seen: Set<string>, pending: ReadonlySet<string>, maxEvents: number) {
  const limit = Math.max(1, Math.trunc(maxEvents));
  if (seen.size <= limit) return;
  for (const eventId of seen) {
    if (seen.size <= limit) break;
    if (!pending.has(eventId)) seen.delete(eventId);
  }
}

export class TerminalStateCache {
  private readonly states = new Map<string, SerializedTerminalState>();

  constructor(
    private readonly maxEntries = MAX_SERIALIZED_TERMINALS,
    private readonly maxBytes = MAX_SERIALIZED_TERMINAL_BYTES,
  ) {}

  get size() {
    return this.states.size;
  }

  get(sessionId: string): SerializedTerminalState | undefined {
    const state = this.states.get(sessionId);
    if (!state) return undefined;
    this.states.delete(sessionId);
    this.states.set(sessionId, state);
    return cloneSerializedTerminalState(state);
  }

  save(sessionId: string, state: SerializedTerminalState): boolean {
    if (!sessionId || utf8ByteLength(state.serialized) > this.maxBytes) return false;
    const rows = normalizeDimension(state.rows);
    const legacyAlternateTimestamp = normalizeTerminalTimestamps([
      { line: 0, ts: state.alternateTimestamp },
    ], 1)[0]?.ts;
    const alternateTimestamps = resizeAlternateTerminalTimestamps(
      state.alternateTimestamps,
      rows,
      legacyAlternateTimestamp,
    );
    const normalized: SerializedTerminalState = {
      serialized: state.serialized,
      cols: normalizeDimension(state.cols),
      rows,
      seenEventIds: state.seenEventIds.slice(-MAX_SERIALIZED_TERMINAL_EVENTS),
      mouseEncoding: normalizeMouseEncoding(state.mouseEncoding),
      timestamps: normalizeTerminalTimestamps(state.timestamps),
      alternateTimestamps: alternateTimestamps.length ? alternateTimestamps : undefined,
      alternateTimestamp: legacyAlternateTimestamp ?? latestTimestamp(alternateTimestamps),
    };
    this.states.delete(sessionId);
    this.states.set(sessionId, normalized);
    while (this.states.size > Math.max(1, this.maxEntries)) {
      const oldest = this.states.keys().next().value;
      if (typeof oldest !== "string") break;
      this.states.delete(oldest);
    }
    return true;
  }

  clear() {
    this.states.clear();
  }
}

export const terminalStateCache = new TerminalStateCache();

function cloneSerializedTerminalState(state: SerializedTerminalState): SerializedTerminalState {
  return {
    ...state,
    seenEventIds: [...state.seenEventIds],
    timestamps: state.timestamps?.map((entry) => ({ ...entry })),
    alternateTimestamps: state.alternateTimestamps?.map((entry) => ({ ...entry })),
  };
}

function latestTimestamp(entries: readonly TerminalTimestampEntry[]): string | undefined {
  return entries.reduce<string | undefined>((latest, entry) => (
    !latest || entry.ts > latest ? entry.ts : latest
  ), undefined);
}

function normalizeDimension(value: number): number {
  return Number.isFinite(value) ? Math.max(1, Math.trunc(value)) : 1;
}

function utf8ByteLength(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

function normalizeMouseEncoding(value: TerminalMouseEncoding | undefined): TerminalMouseEncoding {
  return value === "utf8" || value === "sgr" || value === "urxvt" || value === "sgr-pixels"
    ? value
    : "default";
}
