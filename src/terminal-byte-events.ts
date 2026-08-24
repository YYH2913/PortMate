import { listen } from "@tauri-apps/api/event";
import {
  appendTerminalByteCacheEvents,
  notifyTerminalByteCacheSessions,
  terminalByteCacheSnapshot,
} from "./terminal-byte-state";
import type { TerminalBytesEvent } from "./types";
import type { TerminalLiveEvent } from "./types";

export const TERMINAL_BYTES_EVENT = "portmate-terminal-bytes";
export const TERMINAL_LIVE_EVENT = "portmate-terminal-live";

const TERMINAL_BYTE_FRAME_BATCH_LIMIT = 256;
const TERMINAL_BYTE_FRAME_FLUSH_MS = 16;
const TERMINAL_BYTE_FRAME_MAX_BYTES = 64 * 1024;
type TerminalByteEventSubscriber = (event: TerminalBytesEvent) => void;
type TerminalLiveEventSubscriber = (event: TerminalLiveEvent) => void;

let pendingFrameCount = 0;
const pendingSessionIds = new Set<string>();
const terminalByteEventSubscribers = new Map<string, Set<TerminalByteEventSubscriber>>();
const terminalLiveEventSubscribers = new Map<string, Set<TerminalLiveEventSubscriber>>();
const terminalLiveEventCache = new Map<string, TerminalLiveEvent[]>();
const MAX_TERMINAL_LIVE_EVENT_CACHE = 256;
const MAX_TERMINAL_LIVE_EVENT_CACHE_SESSIONS = 32;
const MAX_TERMINAL_LIVE_EVENT_CACHE_BYTES = 8 * 1024 * 1024;
let terminalLiveEventCacheBytes = 0;
let scheduledFrame: number | null = null;
let scheduledTimer: ReturnType<typeof setTimeout> | null = null;

function flushPendingTerminalByteFrames() {
  if (scheduledFrame !== null && typeof window !== "undefined") {
    window.cancelAnimationFrame(scheduledFrame);
  }
  if (scheduledTimer !== null) clearTimeout(scheduledTimer);
  scheduledFrame = null;
  scheduledTimer = null;
  if (!pendingFrameCount && !pendingSessionIds.size) return;
  const sessionIds = [...pendingSessionIds];
  pendingFrameCount = 0;
  pendingSessionIds.clear();
  notifyTerminalByteCacheSessions(sessionIds);
}

function schedulePendingTerminalByteFrames() {
  if (scheduledFrame !== null || scheduledTimer !== null) return;
  if (typeof window !== "undefined" && typeof window.requestAnimationFrame === "function") {
    scheduledFrame = window.requestAnimationFrame(() => flushPendingTerminalByteFrames());
    return;
  }
  scheduledTimer = setTimeout(() => flushPendingTerminalByteFrames(), TERMINAL_BYTE_FRAME_FLUSH_MS);
}

function normalizeTerminalByteFrame(value: unknown): TerminalBytesEvent | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const frame = value as Partial<TerminalBytesEvent>;
  if (typeof frame.id !== "string" || !frame.id
    || typeof frame.sessionId !== "string" || !frame.sessionId
    || typeof frame.ts !== "string" || !frame.ts
    || (frame.direction !== "inbound" && frame.direction !== "outbound")
    || !["stdout", "stderr", "control", "audit"].includes(frame.stream ?? "")
    || !Array.isArray(frame.bytes)
    || frame.bytes.length > TERMINAL_BYTE_FRAME_MAX_BYTES
    || frame.bytes.some((byte) => !Number.isInteger(byte) || byte < 0 || byte > 0xff)
    || !Number.isSafeInteger(frame.originalLength)
    || (frame.originalLength ?? -1) < frame.bytes.length
    || typeof frame.truncated !== "boolean"
    || (frame.eventId !== undefined && frame.eventId !== null && typeof frame.eventId !== "string")) {
    return null;
  }
  return frame as TerminalBytesEvent;
}

function dispatchTerminalByteFrame(frame: TerminalBytesEvent) {
  const subscribers = terminalByteEventSubscribers.get(frame.sessionId);
  if (!subscribers?.size) return;
  for (const subscriber of [...subscribers]) {
    try {
      subscriber(frame);
    } catch {
      // One terminal view must not delay or suppress delivery to mirrored views.
    }
  }
}

function normalizeTerminalLiveEvent(value: unknown): TerminalLiveEvent | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const packet = value as Partial<TerminalLiveEvent>;
  const event = packet.event;
  if (!event || typeof event !== "object" || Array.isArray(event)
    || typeof event.id !== "string" || !event.id
    || typeof event.sessionId !== "string" || !event.sessionId
    || typeof event.ts !== "string" || !event.ts
    || (event.direction !== "inbound" && event.direction !== "outbound")
    || event.paneId !== event.sessionId + ":main"
    || !event.annotations || typeof event.annotations !== "object" || Array.isArray(event.annotations)
    || (event.text !== undefined && event.text !== null && typeof event.text !== "string")
    || (event.bytesRef !== undefined && event.bytesRef !== null && typeof event.bytesRef !== "string")
    || !["stdout", "stderr", "control", "audit"].includes(event.stream)
    || !Array.isArray(packet.bytes)
    || packet.bytes.length > TERMINAL_BYTE_FRAME_MAX_BYTES
    || packet.bytes.some((byte) => !Number.isInteger(byte) || byte < 0 || byte > 0xff)
    || !Number.isSafeInteger(packet.originalLength)
    || (packet.originalLength ?? -1) < packet.bytes.length
    || typeof packet.truncated !== "boolean"
    || (!packet.truncated && packet.originalLength !== packet.bytes.length)) return null;
  return packet as TerminalLiveEvent;
}

function dispatchTerminalLiveEvent(packet: TerminalLiveEvent) {
  const sessionId = packet.event.sessionId;
  const cache = terminalLiveEventCache.get(sessionId) ?? [];
  if (!cache.some((existing) => existing.event.id === packet.event.id)) {
    cache.push(packet);
    terminalLiveEventCacheBytes += terminalLiveEventSize(packet);
    if (cache.length > MAX_TERMINAL_LIVE_EVENT_CACHE) {
      const removed = cache.splice(0, cache.length - MAX_TERMINAL_LIVE_EVENT_CACHE);
      terminalLiveEventCacheBytes -= removed.reduce((sum, item) => sum + terminalLiveEventSize(item), 0);
    }
    terminalLiveEventCache.delete(sessionId);
    terminalLiveEventCache.set(sessionId, cache);
    trimTerminalLiveEventCache();
  }
  const subscribers = terminalLiveEventSubscribers.get(packet.event.sessionId);
  if (!subscribers?.size) return;
  for (const subscriber of [...subscribers]) {
    try {
      subscriber(packet);
    } catch {
      // A terminal view must not suppress delivery to mirrored views.
    }
  }
}

function queueTerminalLiveEvent(value: unknown): boolean {
  const packet = normalizeTerminalLiveEvent(value);
  if (!packet) return false;
  if (terminalLiveEventCache.get(packet.event.sessionId)?.some((existing) => existing.event.id === packet.event.id)) {
    return false;
  }
  const hasLegacyFrame = terminalByteCacheSnapshot(packet.event.sessionId).frames
    .some((frame) => frame.eventId === packet.event.id);
  const frame: TerminalBytesEvent = {
    id: "live:" + packet.event.id,
    sessionId: packet.event.sessionId,
    ts: packet.event.ts,
    direction: packet.event.direction === "outbound" ? "outbound" : "inbound",
    stream: packet.event.stream,
    bytes: packet.bytes,
    originalLength: packet.originalLength,
    truncated: packet.truncated,
    eventId: packet.event.id,
    canonical: true,
  };
  const changed = hasLegacyFrame
    ? [packet.event.sessionId]
    : appendTerminalByteCacheEvents([frame], false);
  dispatchTerminalLiveEvent(packet);
  pendingFrameCount += 1;
  for (const sessionId of changed) pendingSessionIds.add(sessionId);
  if (pendingFrameCount >= TERMINAL_BYTE_FRAME_BATCH_LIMIT) flushPendingTerminalByteFrames();
  else schedulePendingTerminalByteFrames();
  return true;
}

function queueTerminalByteFrame(value: unknown): boolean {
  const frame = normalizeTerminalByteFrame(value);
  if (!frame) return false;
  if (frame.eventId && terminalLiveEventCache.get(frame.sessionId)?.some((packet) => packet.event.id === frame.eventId)) {
    return false;
  }
  const changed = appendTerminalByteCacheEvents([frame], false);
  if (!changed.length) return false;

  // Live terminal delivery is synchronous and ordered. Hex/split inspector
  // notifications remain frame-batched below so they cannot stall xterm.
  dispatchTerminalByteFrame(frame);
  pendingFrameCount += 1;
  for (const sessionId of changed) pendingSessionIds.add(sessionId);
  // Under a sustained stream, keep latency bounded even when animation frames
  // are throttled (background windows, minimized desktops, or slow WebViews).
  if (pendingFrameCount >= TERMINAL_BYTE_FRAME_BATCH_LIMIT) {
    flushPendingTerminalByteFrames();
  } else {
    schedulePendingTerminalByteFrames();
  }
  return true;
}

export function listenTerminalByteEvents(): Promise<() => void> {
  return listen<TerminalBytesEvent>(TERMINAL_BYTES_EVENT, (event) => {
    queueTerminalByteFrame(event.payload);
  }).then((unlisten) => () => {
    // Commit the final burst before a detached window is closed.
    flushPendingTerminalByteFrames();
    unlisten();
  });
}

export function listenTerminalLiveEvents(): Promise<() => void> {
  return listen<TerminalLiveEvent>(TERMINAL_LIVE_EVENT, (event) => {
    queueTerminalLiveEvent(event.payload);
  }).then((unlisten) => () => {
    flushPendingTerminalByteFrames();
    unlisten();
  });
}

/**
 * Subscribe one terminal view to the window-level byte stream. Existing
 * bounded frames are replayed so lazy terminal mounts cannot miss output.
 */
export function subscribeTerminalByteEvents(
  sessionId: string,
  subscriber: TerminalByteEventSubscriber,
): () => void {
  if (!sessionId) return () => {};
  let subscribers = terminalByteEventSubscribers.get(sessionId);
  if (!subscribers) {
    subscribers = new Set();
    terminalByteEventSubscribers.set(sessionId, subscribers);
  }
  subscribers.add(subscriber);
  for (const frame of terminalByteCacheSnapshot(sessionId).frames) subscriber(frame);

  let disposed = false;
  return () => {
    if (disposed) return;
    disposed = true;
    subscribers?.delete(subscriber);
    if (!subscribers?.size) terminalByteEventSubscribers.delete(sessionId);
  };
}

export function subscribeTerminalLiveEvents(
  sessionId: string,
  subscriber: TerminalLiveEventSubscriber,
): () => void {
  if (!sessionId) return () => {};
  let subscribers = terminalLiveEventSubscribers.get(sessionId);
  if (!subscribers) {
    subscribers = new Set();
    terminalLiveEventSubscribers.set(sessionId, subscribers);
  }
  subscribers.add(subscriber);
  for (const packet of terminalLiveEventCache.get(sessionId) ?? []) subscriber(packet);
  let disposed = false;
  return () => {
    if (disposed) return;
    disposed = true;
    subscribers?.delete(subscriber);
    if (!subscribers?.size) terminalLiveEventSubscribers.delete(sessionId);
  };
}

/** Exposed for deterministic unit tests and controlled window teardown. */
export function flushTerminalByteEventsForTests() {
  flushPendingTerminalByteFrames();
}

/** Exposed for deterministic bridge validation without a native Tauri event loop. */
export function dispatchTerminalByteEventForTests(value: unknown): boolean {
  return queueTerminalByteFrame(value);
}

export function resetTerminalByteEventBridgeForTests() {
  flushPendingTerminalByteFrames();
  terminalByteEventSubscribers.clear();
  terminalLiveEventSubscribers.clear();
  terminalLiveEventCache.clear();
  terminalLiveEventCacheBytes = 0;
  pendingFrameCount = 0;
  pendingSessionIds.clear();
}

function terminalLiveEventSize(packet: TerminalLiveEvent): number {
  return packet.bytes.length + (packet.event.text?.length ?? 0) + 512;
}

function trimTerminalLiveEventCache() {
  while (terminalLiveEventCache.size > MAX_TERMINAL_LIVE_EVENT_CACHE_SESSIONS
    || terminalLiveEventCacheBytes > MAX_TERMINAL_LIVE_EVENT_CACHE_BYTES) {
    const oldest = terminalLiveEventCache.keys().next().value;
    if (typeof oldest !== "string") break;
    const removed = terminalLiveEventCache.get(oldest) ?? [];
    terminalLiveEventCacheBytes -= removed.reduce((sum, item) => sum + terminalLiveEventSize(item), 0);
    terminalLiveEventCache.delete(oldest);
  }
}

export function dispatchTerminalLiveEventForTests(value: unknown): boolean {
  return queueTerminalLiveEvent(value);
}
