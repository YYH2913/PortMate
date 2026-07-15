import type { TerminalMouseEncoding } from "./terminal-mouse";

export type SerializedTerminalState = {
  serialized: string;
  cols: number;
  rows: number;
  seenEventIds: string[];
  mouseEncoding?: TerminalMouseEncoding;
};

export const MAX_SERIALIZED_TERMINALS = 32;
export const MAX_SERIALIZED_TERMINAL_BYTES = 2 * 1024 * 1024;
export const MAX_SERIALIZED_TERMINAL_EVENTS = 4000;

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
    const normalized: SerializedTerminalState = {
      serialized: state.serialized,
      cols: normalizeDimension(state.cols),
      rows: normalizeDimension(state.rows),
      seenEventIds: state.seenEventIds.slice(-MAX_SERIALIZED_TERMINAL_EVENTS),
      mouseEncoding: normalizeMouseEncoding(state.mouseEncoding),
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
  return { ...state, seenEventIds: [...state.seenEventIds] };
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
