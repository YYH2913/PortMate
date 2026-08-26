import type { TerminalBytesEvent } from "./types";

export type TerminalDisplayMode = "text" | "hex" | "split";

export interface TerminalByteFrame extends TerminalBytesEvent {
  startOffset: number;
}

export interface TerminalByteBuffer {
  frames: readonly TerminalByteFrame[];
  capturedBytes: number;
  droppedBytes: number;
  droppedFrames: number;
  nextOffset: number;
  revision: number;
}

export interface TerminalByteBufferLimits {
  maxBytes: number;
  maxFrames: number;
}

export interface TerminalByteRow {
  id: string;
  frameId: string;
  ts: string;
  direction: TerminalByteFrame["direction"];
  stream: TerminalByteFrame["stream"];
  offset: number;
  frameByteOffset: number;
  bytes: number[];
  omittedBytes: number;
}

export interface TerminalByteSelection {
  frameId: string;
  byteIndex: number;
}

export const TERMINAL_BYTE_BYTES_PER_ROW = 16;
export const MAX_TERMINAL_BYTE_BUFFER_BYTES = 1024 * 1024;
export const MAX_TERMINAL_BYTE_BUFFER_FRAMES = 512;
export const MAX_TERMINAL_BYTE_CACHE_SESSIONS = 32;
export const TERMINAL_DISPLAY_MODE_STORAGE_KEY = "portmate.terminalDisplayModes.v1";

export const defaultTerminalByteBufferLimits: TerminalByteBufferLimits = {
  maxBytes: MAX_TERMINAL_BYTE_BUFFER_BYTES,
  maxFrames: MAX_TERMINAL_BYTE_BUFFER_FRAMES,
};

const emptyBuffer: TerminalByteBuffer = Object.freeze({
  frames: Object.freeze([]) as readonly TerminalByteFrame[],
  capturedBytes: 0,
  droppedBytes: 0,
  droppedFrames: 0,
  nextOffset: 0,
  revision: 0,
});

export function emptyTerminalByteBuffer(): TerminalByteBuffer {
  return {
    frames: [],
    capturedBytes: 0,
    droppedBytes: 0,
    droppedFrames: 0,
    nextOffset: 0,
    revision: 0,
  };
}

export function normalizeTerminalDisplayMode(value: unknown): TerminalDisplayMode {
  return value === "hex" || value === "split" ? value : "text";
}

export function readTerminalDisplayMode(
  storage: Pick<Storage, "getItem">,
  viewKey: string,
): TerminalDisplayMode {
  if (!viewKey) return "text";
  try {
    const parsed = JSON.parse(storage.getItem(TERMINAL_DISPLAY_MODE_STORAGE_KEY) ?? "{}") as unknown;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return "text";
    return normalizeTerminalDisplayMode((parsed as Record<string, unknown>)[viewKey]);
  } catch {
    return "text";
  }
}

export function writeTerminalDisplayMode(
  storage: Pick<Storage, "getItem" | "setItem">,
  viewKey: string,
  mode: TerminalDisplayMode,
) {
  if (!viewKey) return;
  let current: Record<string, unknown> = {};
  try {
    const parsed = JSON.parse(storage.getItem(TERMINAL_DISPLAY_MODE_STORAGE_KEY) ?? "{}") as unknown;
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) current = { ...parsed };
  } catch {
    // Replace malformed preferences with the current valid selection.
  }
  current[viewKey] = normalizeTerminalDisplayMode(mode);
  try {
    storage.setItem(TERMINAL_DISPLAY_MODE_STORAGE_KEY, JSON.stringify(current));
  } catch {
    // Display mode remains active for this view even when preferences cannot be persisted.
  }
}

export function appendTerminalByteEvent(
  current: TerminalByteBuffer,
  event: TerminalBytesEvent,
  limits: TerminalByteBufferLimits = defaultTerminalByteBufferLimits,
): TerminalByteBuffer {
  return appendTerminalByteEvents(current, [event], limits);
}

/**
 * Append a transport burst with one bounded frame walk. Serial devices often
 * deliver an interactive echo one byte at a time; cloning and shifting the
 * entire inspector buffer for every byte makes that echo compete with xterm.
 */
export function appendTerminalByteEvents(
  current: TerminalByteBuffer,
  events: readonly TerminalBytesEvent[],
  limits: TerminalByteBufferLimits = defaultTerminalByteBufferLimits,
): TerminalByteBuffer {
  const maxBytes = Math.max(0, Math.trunc(limits.maxBytes));
  const maxFrames = Math.max(0, Math.trunc(limits.maxFrames));
  if (maxBytes === 0 || maxFrames === 0 || !events.length) return current;

  let frames: TerminalByteFrame[] = [...current.frames];
  const frameIds = new Set(frames.map((frame) => frame.id));
  let frameHead = 0;
  let capturedBytes = current.capturedBytes;
  let droppedBytes = current.droppedBytes;
  let droppedFrames = current.droppedFrames;
  let nextOffset = current.nextOffset;
  let revision = current.revision;
  let changed = false;

  for (const event of events) {
    if (!event.id || frameIds.has(event.id)) continue;
    const normalizedBytes = (Array.isArray(event.bytes) ? event.bytes : [])
      .filter((byte) => Number.isInteger(byte) && byte >= 0 && byte <= 0xff)
      .slice(0, maxBytes);
    if (!normalizedBytes.length) continue;

    const originalLength = Math.max(normalizedBytes.length, Math.trunc(event.originalLength) || 0);
    const frame: TerminalByteFrame = {
      ...event,
      bytes: normalizedBytes,
      originalLength,
      truncated: event.truncated || normalizedBytes.length < originalLength,
      startOffset: nextOffset,
    };
    frames.push(frame);
    frameIds.add(frame.id);
    capturedBytes += frame.bytes.length;
    nextOffset += originalLength;
    revision += 1;
    changed = true;

    while (frames.length - frameHead > maxFrames || capturedBytes > maxBytes) {
      const removed = frames[frameHead];
      frameHead += 1;
      if (!removed) break;
      frameIds.delete(removed.id);
      capturedBytes = Math.max(0, capturedBytes - removed.bytes.length);
      droppedBytes += removed.bytes.length;
      droppedFrames += 1;
    }
    if (frameHead >= 256 && frameHead * 2 >= frames.length) {
      frames = frames.slice(frameHead);
      frameHead = 0;
    }
  }

  if (!changed) return current;
  if (frameHead) frames = frames.slice(frameHead);
  return {
    frames,
    capturedBytes,
    droppedBytes,
    droppedFrames,
    nextOffset,
    revision,
  };
}

export function terminalByteRows(
  frames: readonly TerminalByteFrame[],
  bytesPerRow = TERMINAL_BYTE_BYTES_PER_ROW,
): TerminalByteRow[] {
  const width = Math.max(1, Math.trunc(bytesPerRow));
  const rows: TerminalByteRow[] = [];
  for (const frame of frames) {
    for (let frameByteOffset = 0; frameByteOffset < frame.bytes.length; frameByteOffset += width) {
      const bytes = frame.bytes.slice(frameByteOffset, frameByteOffset + width);
      const capturedEnd = frameByteOffset + bytes.length;
      rows.push({
        id: `${frame.id}:${frameByteOffset}`,
        frameId: frame.id,
        ts: frame.ts,
        direction: frame.direction,
        stream: frame.stream,
        offset: frame.startOffset + frameByteOffset,
        frameByteOffset,
        bytes,
        omittedBytes: capturedEnd === frame.bytes.length
          ? Math.max(0, frame.originalLength - frame.bytes.length)
          : 0,
      });
    }
  }
  return rows;
}

export function terminalByteAscii(bytes: readonly number[]): string {
  return bytes.map((byte) => {
    if (byte === 0x0d) return "\\r";
    if (byte === 0x0a) return "\\n";
    if (byte === 0x09) return "\\t";
    return byte >= 0x20 && byte <= 0x7e ? String.fromCharCode(byte) : ".";
  }).join("");
}

export function terminalByteCellCharacter(byte: number): string {
  if (byte >= 0x21 && byte <= 0x7e) return String.fromCharCode(byte);
  if (byte === 0x20) return "\u00a0";
  return ".";
}

export function terminalByteCellLabel(byte: number): string {
  if (byte === 0x0d) return "CR \\r";
  if (byte === 0x0a) return "LF \\n";
  if (byte === 0x09) return "TAB \\t";
  if (byte === 0x00) return "NUL";
  if (byte >= 0x20 && byte <= 0x7e) return String.fromCharCode(byte);
  return `非打印字节 0x${terminalByteHex(byte)}`;
}

export function terminalByteHex(byte: number): string {
  return byte.toString(16).padStart(2, "0").toUpperCase();
}

export function terminalByteSelectionKey(selection: TerminalByteSelection): string {
  return `${selection.frameId}:${selection.byteIndex}`;
}

export function sameTerminalByteSelection(
  left: TerminalByteSelection | null,
  right: TerminalByteSelection | null,
): boolean {
  return Boolean(left && right && left.frameId === right.frameId && left.byteIndex === right.byteIndex);
}

export function terminalByteSelectionAt(
  frames: readonly TerminalByteFrame[],
  absoluteIndex: number,
): TerminalByteSelection | null {
  const total = frames.reduce((sum, frame) => sum + frame.bytes.length, 0);
  if (!total) return null;
  let remaining = Math.min(total - 1, Math.max(0, Math.trunc(absoluteIndex)));
  for (const frame of frames) {
    if (remaining < frame.bytes.length) return { frameId: frame.id, byteIndex: remaining };
    remaining -= frame.bytes.length;
  }
  return null;
}

export function terminalByteSelectionPosition(
  frames: readonly TerminalByteFrame[],
  selection: TerminalByteSelection | null,
): number | null {
  if (!selection) return null;
  let position = 0;
  for (const frame of frames) {
    if (frame.id === selection.frameId) {
      if (selection.byteIndex < 0 || selection.byteIndex >= frame.bytes.length) return null;
      return position + selection.byteIndex;
    }
    position += frame.bytes.length;
  }
  return null;
}

export function moveTerminalByteSelection(
  frames: readonly TerminalByteFrame[],
  selection: TerminalByteSelection | null,
  delta: number,
): TerminalByteSelection | null {
  const current = terminalByteSelectionPosition(frames, selection);
  const total = frames.reduce((sum, frame) => sum + frame.bytes.length, 0);
  if (!total) return null;
  const target = current === null
    ? (delta < 0 ? total - 1 : 0)
    : Math.min(total - 1, Math.max(0, current + Math.trunc(delta)));
  return terminalByteSelectionAt(frames, target);
}

export function terminalByteSelectionRowIndex(
  frames: readonly TerminalByteFrame[],
  selection: TerminalByteSelection | null,
  bytesPerRow = TERMINAL_BYTE_BYTES_PER_ROW,
): number | null {
  if (!selection) return null;
  const width = Math.max(1, Math.trunc(bytesPerRow));
  let row = 0;
  for (const frame of frames) {
    if (frame.id === selection.frameId) {
      if (selection.byteIndex < 0 || selection.byteIndex >= frame.bytes.length) return null;
      return row + Math.floor(selection.byteIndex / width);
    }
    row += Math.ceil(frame.bytes.length / width);
  }
  return null;
}

export function terminalByteFollowForScroll(
  scrollTop: number,
  clientHeight: number,
  scrollHeight: number,
  tolerance = 48,
): boolean {
  return scrollHeight - Math.max(0, scrollTop) - Math.max(0, clientHeight)
    <= Math.max(0, tolerance);
}

export function terminalByteBufferStats(buffer: TerminalByteBuffer) {
  let rxBytes = 0;
  let txBytes = 0;
  let omittedBytes = 0;
  for (const frame of buffer.frames) {
    if (frame.direction === "inbound") rxBytes += frame.originalLength;
    else txBytes += frame.originalLength;
    omittedBytes += Math.max(0, frame.originalLength - frame.bytes.length);
  }
  return { rxBytes, txBytes, omittedBytes };
}

type TerminalByteCacheEntry = {
  sessionId: string;
  buffer: TerminalByteBuffer;
  frameIds: Set<string>;
  eventIds: Set<string>;
  listeners: Set<() => void>;
  touchedAt: number;
};

const terminalByteCache = new Map<string, TerminalByteCacheEntry>();
let cacheClock = 0;

function terminalByteCacheEntry(sessionId: string): TerminalByteCacheEntry {
  const existing = terminalByteCache.get(sessionId);
  if (existing) {
    existing.touchedAt = ++cacheClock;
    return existing;
  }
  const entry = {
    sessionId,
    buffer: emptyTerminalByteBuffer(),
    frameIds: new Set<string>(),
    eventIds: new Set<string>(),
    listeners: new Set<() => void>(),
    touchedAt: ++cacheClock,
  };
  terminalByteCache.set(sessionId, entry);
  trimTerminalByteCache();
  return entry;
}

function trimTerminalByteCache() {
  while (terminalByteCache.size > MAX_TERMINAL_BYTE_CACHE_SESSIONS) {
    const candidate = [...terminalByteCache.entries()]
      .filter(([, entry]) => entry.listeners.size === 0)
      .sort((left, right) => left[1].touchedAt - right[1].touchedAt)[0];
    if (!candidate) return;
    terminalByteCache.delete(candidate[0]);
  }
}

export function terminalByteCacheSnapshot(sessionId: string): TerminalByteBuffer {
  if (!sessionId) return emptyBuffer;
  return terminalByteCache.get(sessionId)?.buffer ?? emptyBuffer;
}

export function terminalByteCacheHasFrameId(sessionId: string, frameId: string): boolean {
  return Boolean(sessionId && frameId && terminalByteCache.get(sessionId)?.frameIds.has(frameId));
}

export function terminalByteCacheHasEventId(sessionId: string, eventId: string): boolean {
  return Boolean(sessionId && eventId && terminalByteCache.get(sessionId)?.eventIds.has(eventId));
}

export function appendTerminalByteCacheEvent(event: TerminalBytesEvent): TerminalByteBuffer {
  appendTerminalByteCacheEvents([event]);
  return event.sessionId ? terminalByteCacheSnapshot(event.sessionId) : emptyBuffer;
}

/**
 * Apply a burst of wire frames while notifying each session's subscribers
 * only once. Transport events can arrive much faster than a browser frame;
 * batching avoids rebuilding the Hex view for every read() chunk.
 */
export function appendTerminalByteCacheEvents(
  events: readonly TerminalBytesEvent[],
  notify = true,
): readonly string[] {
  const changedEntries = new Set<TerminalByteCacheEntry>();
  const eventsBySession = new Map<string, TerminalBytesEvent[]>();
  for (const event of events) {
    if (!event.sessionId) continue;
    const sessionEvents = eventsBySession.get(event.sessionId);
    if (sessionEvents) sessionEvents.push(event);
    else eventsBySession.set(event.sessionId, [event]);
  }
  for (const [sessionId, sessionEvents] of eventsBySession) {
    const entry = terminalByteCacheEntry(sessionId);
    const next = appendTerminalByteEvents(entry.buffer, sessionEvents);
    if (next === entry.buffer) continue;
    entry.buffer = next;
    entry.frameIds = new Set(next.frames.map((frame) => frame.id));
    entry.eventIds = new Set(next.frames.flatMap((frame) => frame.eventId ? [frame.eventId] : []));
    changedEntries.add(entry);
  }
  const changedSessionIds = [...changedEntries].map((entry) => entry.sessionId);
  if (notify) notifyTerminalByteCacheSessions(changedSessionIds);
  return changedSessionIds;
}

export function notifyTerminalByteCacheSessions(sessionIds: readonly string[]): void {
  const notified = new Set<TerminalByteCacheEntry>();
  for (const sessionId of sessionIds) {
    const entry = terminalByteCache.get(sessionId);
    if (entry) notified.add(entry);
  }
  for (const entry of notified) {
    for (const listener of entry.listeners) listener();
  }
}

export function clearTerminalByteCache(sessionId: string): TerminalByteBuffer {
  if (!sessionId) return emptyBuffer;
  const entry = terminalByteCacheEntry(sessionId);
  entry.buffer = { ...emptyTerminalByteBuffer(), revision: entry.buffer.revision + 1 };
  entry.frameIds.clear();
  entry.eventIds.clear();
  for (const listener of entry.listeners) listener();
  return entry.buffer;
}

export function subscribeTerminalByteCache(sessionId: string, listener: () => void): () => void {
  if (!sessionId) return () => {};
  const entry = terminalByteCacheEntry(sessionId);
  entry.listeners.add(listener);
  return () => {
    entry.listeners.delete(listener);
    trimTerminalByteCache();
  };
}

export function resetTerminalByteCacheForTests() {
  terminalByteCache.clear();
  cacheClock = 0;
}
