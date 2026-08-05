import { listen } from "@tauri-apps/api/event";
import { appendTerminalByteCacheEvent } from "./terminal-byte-state";
import type { TerminalBytesEvent } from "./types";

export const TERMINAL_BYTES_EVENT = "portmate-terminal-bytes";

export function listenTerminalByteEvents(): Promise<() => void> {
  return listen<TerminalBytesEvent>(TERMINAL_BYTES_EVENT, (event) => {
    appendTerminalByteCacheEvent(event.payload);
  });
}
