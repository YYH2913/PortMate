/** One deadline timer per terminal, not one timer per legacy text packet. */
export class TerminalRawCorrelator<T> {
  private readonly pending = new Map<string, { value: T; deadline: number }>();
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor(private readonly waitMs: number, private readonly expire: (values: T[]) => void) {}

  add(id: string, value: T): void {
    // Keep insertion/deadline order even when a caller replaces a key.
    this.pending.delete(id);
    this.pending.set(id, { value, deadline: performance.now() + this.waitMs });
    this.schedule();
  }

  take(id: string): T | undefined {
    const entry = this.pending.get(id);
    if (!entry) return undefined;
    this.pending.delete(id);
    if (!this.pending.size && this.timer !== null) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    return entry.value;
  }

  dispose(): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
    this.pending.clear();
  }

  private schedule(): void {
    if (this.timer !== null) return;
    const first = this.pending.values().next().value;
    if (!first) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      const now = performance.now();
      const expired: T[] = [];
      for (const [id, entry] of this.pending) {
        if (entry.deadline > now) break;
        this.pending.delete(id);
        expired.push(entry.value);
      }
      if (expired.length) this.expire(expired);
      this.schedule();
    }, Math.max(0, first.deadline - performance.now()));
  }
}
