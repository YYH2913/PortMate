import type { IDisposable, IMarker } from "@xterm/xterm";
import type { TerminalTimestampEntry, VisibleTerminalTimestamp } from "./terminal-timestamp-state";

type Entry = { marker: IMarker; ts: string; subscription: IDisposable };

/**
 * Ordered sparse timestamps backed by xterm's public markers. Buffer trim,
 * insert, delete and reflow move surviving markers without changing their
 * relative order. Ordinary output therefore needs no full-history sort/scan.
 * Disposal notifications trigger cleanup only when the buffer actually changes.
 */
export class TerminalTimestampIndex {
  private entries: Entry[] = [];
  private head = 0;
  private disposed = 0;
  anchor: string | null = null;

  constructor(private readonly limit: number) {}

  get size(): number {
    this.compact();
    return this.entries.length - this.head;
  }

  has(line: number): boolean {
    this.compact();
    return this.entries[this.lowerBound(line)]?.marker.line === line;
  }

  timestampAt(line: number): string | null {
    this.compact();
    const index = this.upperBound(line) - 1;
    return index >= this.head ? this.entries[index].ts : this.anchor;
  }

  /** Takes ownership, including disposing a duplicate marker. */
  add(marker: IMarker, ts: string): boolean {
    this.compact();
    if (marker.isDisposed || marker.line < 0) { marker.dispose(); return false; }
    const index = this.lowerBound(marker.line);
    if (this.entries[index]?.marker.line === marker.line) { marker.dispose(); return false; }
    const subscription = marker.onDispose(() => { this.disposed += 1; });
    this.entries.splice(index, 0, { marker, ts, subscription });
    while (this.entries.length - this.head > Math.max(1, this.limit)) {
      const removed = this.entries[this.head++];
      this.anchor = removed.ts;
      removed.subscription.dispose();
      removed.marker.dispose();
    }
    this.releasePrefix();
    return true;
  }

  removeRange(firstLine: number, lastLine: number): void {
    this.compact();
    const first = this.lowerBound(firstLine);
    const end = this.upperBound(lastLine);
    if (end <= first) return;
    // Redraws normally affect only the screen at the tail of the buffer. Splice
    // that range instead of copying every timestamp in the retained scrollback.
    const removed = this.entries.splice(first, end - first);
    for (const entry of removed) {
      entry.subscription.dispose();
      entry.marker.dispose();
    }
  }

  visible(viewportY: number, rows: number): VisibleTerminalTimestamp[] {
    this.compact();
    const firstLine = Math.max(0, Math.trunc(viewportY) || 0);
    const count = Math.max(0, Math.trunc(rows) || 0);
    if (!count) return [];
    const start = this.upperBound(firstLine);
    let ts = start > this.head ? this.entries[start - 1].ts : this.anchor;
    const result: VisibleTerminalTimestamp[] = ts ? [{ line: firstLine, row: 0, ts }] : [];
    for (let index = start; index < this.entries.length; index += 1) {
      const entry = this.entries[index];
      const line = entry.marker.line;
      if (line >= firstLine + count) break;
      if (entry.ts === ts) continue;
      ts = entry.ts;
      result.push({ line, row: line - firstLine, ts });
    }
    return result;
  }

  snapshot(): TerminalTimestampEntry[] {
    this.compact();
    return this.entries.slice(this.head).map(({ marker, ts }) => ({ line: marker.line, ts }));
  }

  dispose(): void {
    for (const entry of this.entries.slice(this.head)) {
      entry.subscription.dispose();
      entry.marker.dispose();
    }
    this.entries = [];
    this.head = 0;
    this.disposed = 0;
    this.anchor = null;
  }

  private lowerBound(line: number): number {
    let low = this.head;
    let high = this.entries.length;
    // The hot path appends at the end. Restoring N rows is O(N), not O(N²).
    if (high > low && this.entries[high - 1].marker.line < line) return high;
    while (low < high) {
      const middle = low + Math.floor((high - low) / 2);
      if (this.entries[middle].marker.line < line) low = middle + 1;
      else high = middle;
    }
    return low;
  }

  private upperBound(line: number): number {
    let low = this.head;
    let high = this.entries.length;
    while (low < high) {
      const middle = low + Math.floor((high - low) / 2);
      if (this.entries[middle].marker.line <= line) low = middle + 1;
      else high = middle;
    }
    return low;
  }

  private compact(): void {
    if (!this.disposed) return;
    // Scrollback eviction disposes a prefix. Advance a cursor rather than
    // shifting the whole array for each scrolled-off row.
    while (this.head < this.entries.length && this.entries[this.head].marker.isDisposed) {
      const entry = this.entries[this.head++];
      if (entry.marker.line < 0) this.anchor = entry.ts;
      entry.subscription.dispose();
      this.disposed -= 1;
    }
    if (this.disposed) {
      if (this.disposed > 64) {
        // A large resize/reflow can delete rows throughout the scrollback.
        this.entries = this.entries.slice(this.head).filter(entry => {
          if (!entry.marker.isDisposed) return true;
          entry.subscription.dispose();
          return false;
        });
        this.head = 0;
        this.disposed = 0;
      } else {
        // CSI erase/delete usually touches the current screen near the tail.
        // Stop as soon as those markers are removed, without inspecting the
        // preceding history on every prompt redraw or wrapped-line backspace.
        for (let index = this.entries.length - 1; index >= this.head && this.disposed; index -= 1) {
          const entry = this.entries[index];
          if (!entry.marker.isDisposed) continue;
          this.entries.splice(index, 1);
          entry.subscription.dispose();
          this.disposed -= 1;
        }
      }
    }
    this.releasePrefix();
  }

  private releasePrefix(): void {
    if (this.head >= 256 && this.head * 2 >= this.entries.length) {
      this.entries = this.entries.slice(this.head);
      this.head = 0;
    }
  }
}
