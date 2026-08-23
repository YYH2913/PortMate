import { listen } from "@tauri-apps/api/event";
import { appendTerminalByteCacheEvents, notifyTerminalByteCacheSessions } from "./terminal-byte-state";
import type { TerminalBytesEvent } from "./types";

export const TERMINAL_BYTES_EVENT = "portmate-terminal-bytes";

const TERMINAL_BYTE_FRAME_BATCH_LIMIT = 256;
const TERMINAL_BYTE_FRAME_FLUSH_MS = 16;
let pendingFrameCount = 0;
const pendingSessionIds = new Set<string>();
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

function queueTerminalByteFrame(frame: TerminalBytesEvent) {
  pendingFrameCount += 1;
  const changed = appendTerminalByteCacheEvents([frame], false);
  for (const sessionId of changed) pendingSessionIds.add(sessionId);
  // Under a sustained stream, keep latency bounded even when animation frames
  // are throttled (background windows, minimized desktops, or slow WebViews).
  if (pendingFrameCount >= TERMINAL_BYTE_FRAME_BATCH_LIMIT) {
    flushPendingTerminalByteFrames();
  } else {
    schedulePendingTerminalByteFrames();
  }
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

/** Exposed for deterministic unit tests and controlled window teardown. */
export function flushTerminalByteEventsForTests() {
  flushPendingTerminalByteFrames();
}
