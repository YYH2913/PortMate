import { listen } from "@tauri-apps/api/event";
import {
  appendTerminalByteCacheEvents,
  notifyTerminalByteCacheSessions,
  terminalByteCacheSnapshot,
} from "./terminal-byte-state";
import type { TerminalBytesEvent } from "./types";

export const TERMINAL_BYTES_EVENT = "portmate-terminal-bytes";

const TERMINAL_BYTE_FRAME_BATCH_LIMIT = 256;
const TERMINAL_BYTE_FRAME_FLUSH_MS = 16;
const TERMINAL_BYTE_FRAME_MAX_BYTES = 64 * 1024;
type TerminalByteEventSubscriber = (event: TerminalBytesEvent) => void;

let pendingFrameCount = 0;
const pendingSessionIds = new Set<string>();
const terminalByteEventSubscribers = new Map<string, Set<TerminalByteEventSubscriber>>();
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

function queueTerminalByteFrame(value: unknown): boolean {
  const frame = normalizeTerminalByteFrame(value);
  if (!frame) return false;
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
  pendingFrameCount = 0;
  pendingSessionIds.clear();
}
