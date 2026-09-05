import { normalizeTerminalTimestampValue } from "./terminal-timestamp-state";
import type { SessionEvent } from "./types";

/** Historical ANSI must never be appended behind output already delivered live. */
export class TerminalReplayBoundary {
  timestamp: string | null;

  constructor(timestamp?: string | null) {
    this.timestamp = normalizeTerminalTimestampValue(timestamp);
  }

  accept(event: SessionEvent, replay: boolean): boolean {
    if (event.direction === "outbound") return true;
    const timestamp = normalizeTerminalTimestampValue(event.ts);
    if (!timestamp) return !replay || this.timestamp === null;
    if (replay && this.timestamp && timestamp < this.timestamp) return false;
    // Wall-clock adjustments must not suppress fresh transport bytes.
    if (!this.timestamp || timestamp > this.timestamp) this.timestamp = timestamp;
    return true;
  }
}

export function terminalHistorySuffix(
  events: readonly SessionEvent[],
  previousIds: readonly string[],
  seen: ReadonlySet<string>,
): readonly SessionEvent[] {
  const previous = new Set(previousIds);
  for (let index = events.length - 1; index >= 0; index -= 1) {
    if (previous.has(events[index].id) || seen.has(events[index].id)) return events.slice(index + 1);
  }
  return events;
}
