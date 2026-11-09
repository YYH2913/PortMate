import { t } from "./i18n";
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

export interface TerminalByteRowIndex {
  frames: readonly TerminalByteFrame[];
  bytesPerRow: number;
  /** Prefix row counts, one per transport frame; no byte-level expansion. */
  starts: readonly number[];
  rowCount: number;
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
  const capture = new TerminalByteCapture(current, limits);
  capture.append(events);
  return capture.snapshot();
}

/** Mutable bounded capture, immutable snapshots only when a reader needs one. */
export class TerminalByteCapture {
  private frames: Array<TerminalByteFrame | undefined>;
  private head = 0;
  readonly frameIds: Set<string>;
  private readonly eventCounts = new Map<string, number>();
  private cached: TerminalByteBuffer | null;
  private capturedBytes: number;
  private droppedBytes: number;
  private droppedFrames: number;
  private nextOffset: number;
  private revision: number;
  private readonly maxBytes: number;
  private readonly maxFrames: number;

  constructor(current = emptyTerminalByteBuffer(), limits = defaultTerminalByteBufferLimits) {
    this.frames = [...current.frames];
    this.frameIds = new Set(current.frames.map(frame => frame.id));
    for (const frame of current.frames) this.addEventId(frame.eventId);
    this.cached = current;
    this.capturedBytes = current.capturedBytes;
    this.droppedBytes = current.droppedBytes;
    this.droppedFrames = current.droppedFrames;
    this.nextOffset = current.nextOffset;
    this.revision = current.revision;
    this.maxBytes = Math.max(0, Math.trunc(limits.maxBytes));
    this.maxFrames = Math.max(0, Math.trunc(limits.maxFrames));
  }

  hasEventId(id: string): boolean { return this.eventCounts.has(id); }

  append(events: readonly TerminalBytesEvent[]): boolean {
    if (!this.maxBytes || !this.maxFrames) return false;
    let changed = false;
    for (const event of events) {
      if (!event.id || this.frameIds.has(event.id)) continue;
      const bytes = event.bytes instanceof Uint8Array
        ? event.bytes.length <= this.maxBytes ? event.bytes : event.bytes.slice(0, this.maxBytes)
        : Uint8Array.from((Array.isArray(event.bytes) ? event.bytes : [])
          .filter(byte => Number.isInteger(byte) && byte >= 0 && byte <= 0xff)
          .slice(0, this.maxBytes));
      if (!bytes.length) continue;
      const originalLength = Math.max(bytes.length, Math.trunc(event.originalLength) || 0);
      const frame: TerminalByteFrame = { ...event, bytes, originalLength,
        truncated: event.truncated || bytes.length < originalLength, startOffset: this.nextOffset };
      this.frames.push(frame);
      this.frameIds.add(frame.id);
      this.addEventId(frame.eventId);
      this.capturedBytes += bytes.length;
      this.nextOffset += originalLength;
      this.revision++;
      changed = true;
      while (this.frames.length - this.head > this.maxFrames || this.capturedBytes > this.maxBytes) {
        const removed = this.frames[this.head]!;
        this.frames[this.head++] = undefined; // Release evicted byte storage immediately.
        this.frameIds.delete(removed.id);
        if (removed.eventId) {
          const count = this.eventCounts.get(removed.eventId)! - 1;
          if (count) this.eventCounts.set(removed.eventId, count);
          else this.eventCounts.delete(removed.eventId);
        }
        this.capturedBytes -= removed.bytes.length;
        this.droppedBytes += removed.bytes.length;
        this.droppedFrames++;
      }
      if (this.head >= 256 && this.head * 2 >= this.frames.length) {
        this.frames = this.frames.slice(this.head);
        this.head = 0;
      }
    }
    if (changed) this.cached = null;
    return changed;
  }

  snapshot(): TerminalByteBuffer {
    return this.cached ??= { frames: this.frames.slice(this.head) as TerminalByteFrame[],
      capturedBytes: this.capturedBytes, droppedBytes: this.droppedBytes, droppedFrames: this.droppedFrames,
      nextOffset: this.nextOffset, revision: this.revision };
  }

  clear(): TerminalByteBuffer {
    this.frames = [];
    this.head = 0;
    this.frameIds.clear();
    this.eventCounts.clear();
    this.capturedBytes = this.droppedBytes = this.droppedFrames = this.nextOffset = 0;
    this.revision++;
    return this.cached = { ...emptyTerminalByteBuffer(), revision: this.revision };
  }

  private addEventId(id: string | null | undefined): void {
    if (id) this.eventCounts.set(id, (this.eventCounts.get(id) ?? 0) + 1);
  }
}

export function terminalByteRows(
  frames: readonly TerminalByteFrame[],
  bytesPerRow = TERMINAL_BYTE_BYTES_PER_ROW,
): TerminalByteRow[] {
  const index = indexTerminalByteRows(frames, bytesPerRow);
  return terminalByteRowRange(index, 0, index.rowCount);
}

export function indexTerminalByteRows(
  frames: readonly TerminalByteFrame[],
  bytesPerRow = TERMINAL_BYTE_BYTES_PER_ROW,
): TerminalByteRowIndex {
  const width = Number.isFinite(bytesPerRow) ? Math.max(1, Math.trunc(bytesPerRow)) : TERMINAL_BYTE_BYTES_PER_ROW;
  const starts: number[] = [];
  let rowCount = 0;
  for (const frame of frames) {
    starts.push(rowCount);
    rowCount += Math.ceil(frame.bytes.length / width);
  }
  return { frames, bytesPerRow: width, starts, rowCount };
}

/** Materialize only the viewport, not 65k–131k rows for a full 1 MiB capture. */
export function terminalByteRowRange(index: TerminalByteRowIndex, firstRow: number, count: number): TerminalByteRow[] {
  const start = Math.max(0, Math.trunc(firstRow) || 0);
  const end = Math.min(index.rowCount, start + Math.max(0, Math.trunc(count) || 0));
  const rows: TerminalByteRow[] = [];
  if (end <= start) return rows;
  let low = 0, high = index.frames.length;
  while (low < high) {
    const middle = low + Math.floor((high - low) / 2);
    if (index.starts[middle] <= start) low = middle + 1;
    else high = middle;
  }
  const width = index.bytesPerRow;
  for (let frameIndex = Math.max(0, low - 1); frameIndex < index.frames.length; frameIndex++) {
    const frame = index.frames[frameIndex];
    const frameStart = index.starts[frameIndex];
    if (frameStart >= end) break;
    const firstByte = Math.max(0, start - frameStart) * width;
    const lastByte = Math.min(frame.bytes.length, (end - frameStart) * width);
    for (let frameByteOffset = firstByte; frameByteOffset < lastByte; frameByteOffset += width) {
      const bytes = Array.from(frame.bytes.slice(frameByteOffset, frameByteOffset + width));
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
  return t("non-printable-byte-0x", [terminalByteHex(byte)]);
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
  capture: TerminalByteCapture;
  listeners: Set<() => void>;
  presentationListeners: Set<() => void>;
  presentation: TerminalByteBuffer | null;
  presentationTimer: ReturnType<typeof setTimeout> | null;
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
    capture: new TerminalByteCapture(),
    listeners: new Set<() => void>(),
    presentationListeners: new Set<() => void>(),
    presentation: null,
    presentationTimer: null,
    touchedAt: ++cacheClock,
  };
  terminalByteCache.set(sessionId, entry);
  trimTerminalByteCache();
  return entry;
}

function trimTerminalByteCache() {
  while (terminalByteCache.size > MAX_TERMINAL_BYTE_CACHE_SESSIONS) {
    const candidate = [...terminalByteCache.entries()]
      .filter(([, entry]) => entry.listeners.size === 0 && entry.presentationListeners.size === 0)
      .sort((left, right) => left[1].touchedAt - right[1].touchedAt)[0];
    if (!candidate) return;
    terminalByteCache.delete(candidate[0]);
  }
}

export function terminalByteCacheSnapshot(sessionId: string): TerminalByteBuffer {
  if (!sessionId) return emptyBuffer;
  return terminalByteCache.get(sessionId)?.capture.snapshot() ?? emptyBuffer;
}

export function terminalByteCacheHasFrameId(sessionId: string, frameId: string): boolean {
  return Boolean(sessionId && frameId && terminalByteCache.get(sessionId)?.capture.frameIds.has(frameId));
}

export function terminalByteCacheHasEventId(sessionId: string, eventId: string): boolean {
  return Boolean(sessionId && eventId && terminalByteCache.get(sessionId)?.capture.hasEventId(eventId));
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
    if (!entry.capture.append(sessionEvents)) continue;
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
    if (entry.presentationListeners.size && entry.presentationTimer === null) {
      // Auxiliary counters/Hex refresh at 10 Hz. Raw capture and live terminal
      // delivery are independent and still consume every packet immediately.
      entry.presentationTimer = setTimeout(() => publishTerminalBytePresentation(entry), 100);
    }
  }
}

function publishTerminalBytePresentation(entry: TerminalByteCacheEntry): void {
  if (entry.presentationTimer !== null) clearTimeout(entry.presentationTimer);
  entry.presentationTimer = null;
  entry.presentation = entry.capture.snapshot();
  for (const listener of entry.presentationListeners) listener();
}

export function terminalBytePresentationSnapshot(sessionId: string): TerminalByteBuffer {
  const entry = terminalByteCache.get(sessionId);
  return entry?.presentation ?? entry?.capture.snapshot() ?? emptyBuffer;
}

export function subscribeTerminalBytePresentation(sessionId: string, listener: () => void): () => void {
  if (!sessionId) return () => {};
  const entry = terminalByteCacheEntry(sessionId);
  entry.presentation ??= entry.capture.snapshot();
  entry.presentationListeners.add(listener);
  return () => {
    entry.presentationListeners.delete(listener);
    if (!entry.presentationListeners.size) {
      if (entry.presentationTimer !== null) clearTimeout(entry.presentationTimer);
      entry.presentationTimer = null;
      entry.presentation = null;
    }
    trimTerminalByteCache();
  };
}

export function clearTerminalByteCache(sessionId: string): TerminalByteBuffer {
  if (!sessionId) return emptyBuffer;
  const entry = terminalByteCacheEntry(sessionId);
  const cleared = entry.capture.clear();
  for (const listener of entry.listeners) listener();
  publishTerminalBytePresentation(entry);
  return cleared;
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
  for (const entry of terminalByteCache.values()) {
    if (entry.presentationTimer !== null) clearTimeout(entry.presentationTimer);
  }
  terminalByteCache.clear();
  cacheClock = 0;
}
