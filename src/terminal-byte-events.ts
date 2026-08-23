import { listen } from "@tauri-apps/api/event";
import { appendTerminalByteCacheEvents } from "./terminal-byte-state";
import type { TerminalBytesEvent } from "./types";

export const TERMINAL_BYTES_EVENT = "portmate-terminal-bytes";

const TERMINAL_BYTE_FRAME_BATCH_LIMIT = 256;
const TERMINAL_BYTE_FRAME_FLUSH_MS = 16;
let pendingFrames: TerminalBytesEvent[] = [];
let scheduledFrame: number | null = null;
let scheduledTimer: ReturnType<typeof setTimeout> | null = null;

function flushPendingTerminalByteFrames() {
  if (scheduledFrame !== null && typeof window !== "undefined") {
    window.cancelAnimationFrame(scheduledFrame);
  }
  if (scheduledTimer !== null) clearTimeout(scheduledTimer);
  scheduledFrame = null;
  scheduledTimer = null;
  if (!pendingFrames.length) return;
  const frames = pendingFrames;
  pendingFrames = [];
  appendTerminalByteCacheEvents(frames);
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
  pendingFrames.push(frame);
  // Under a sustained stream, keep latency bounded even when animation frames
  // are throttled (background windows, minimized desktops, or slow WebViews).
  if (pendingFrames.length >= TERMINAL_BYTE_FRAME_BATCH_LIMIT) {
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
