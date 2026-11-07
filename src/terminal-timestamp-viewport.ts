import type { VisibleTerminalTimestamp } from "./terminal-timestamp-state";

export type TerminalTimestampViewport = {
  bufferType: "normal" | "alternate";
  screenTop: number;
  cellHeight: number;
  entries: VisibleTerminalTimestamp[];
};

/** Timestamp animation must not rerender completion, input and the whole terminal. */
export class TerminalTimestampViewportStore {
  private value: TerminalTimestampViewport = { bufferType: "normal", screenTop: 0, cellHeight: 0, entries: [] };
  private listeners = new Set<() => void>();
  getSnapshot = () => this.value;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  publish(next: TerminalTimestampViewport): void {
    const current = this.value;
    if (current.bufferType === next.bufferType && Math.abs(current.screenTop - next.screenTop) < .25
      && Math.abs(current.cellHeight - next.cellHeight) < .25 && current.entries.length === next.entries.length
      && current.entries.every((entry, index) => entry.line === next.entries[index].line
        && entry.row === next.entries[index].row && entry.ts === next.entries[index].ts)) return;
    this.value = next;
    for (const listener of this.listeners) listener();
  }
}
