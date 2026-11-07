/** Bounded, non-blocking startup work. Queue keys prevent duplicate attempts. */
export class BackgroundConnectionQueue {
  private pending = new Map<string, () => Promise<void>>();
  private running = new Set<string>();
  private timer: ReturnType<typeof setTimeout> | null = null;
  private disposed = false;

  constructor(private readonly concurrency = 4) {}

  enqueue(id: string, connect: () => Promise<void>): void {
    if (this.disposed || this.pending.has(id) || this.running.has(id)) return;
    this.pending.set(id, connect);
    this.schedule();
  }

  cancel(id: string): void { this.pending.delete(id); }

  dispose(): void {
    this.disposed = true;
    this.pending.clear();
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
  }

  private schedule(): void {
    if (this.disposed || this.timer !== null || !this.pending.size || this.running.size >= this.concurrency) return;
    // Yield to paint/input between groups instead of running startup work in
    // React's effect/microtask and monopolizing the initial visible frame.
    this.timer = setTimeout(() => { this.timer = null; this.drain(); }, 0);
  }

  private drain(): void {
    while (!this.disposed && this.pending.size && this.running.size < this.concurrency) {
      const [id, connect] = this.pending.entries().next().value!;
      this.pending.delete(id);
      this.running.add(id);
      void Promise.resolve().then(connect).catch(() => {
        // Connection state/logs own the diagnosis; startup never raises a dialog.
      }).finally(() => { this.running.delete(id); this.schedule(); });
    }
  }
}
