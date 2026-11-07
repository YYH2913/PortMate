/** Yield between bounded parser bursts without browsers' nested 4 ms timers. */
export class TerminalTaskScheduler {
  private readonly channel = typeof MessageChannel === "undefined" ? null : new MessageChannel();
  private timer: ReturnType<typeof setTimeout> | null = null;
  private scheduled = false;
  private disposed = false;

  constructor(private readonly run: () => void) {
    if (this.channel) this.channel.port1.onmessage = () => this.flush();
  }

  get pending(): boolean { return this.scheduled; }

  schedule(): void {
    if (this.disposed || this.scheduled) return;
    this.scheduled = true;
    if (this.channel) this.channel.port2.postMessage(null);
    else this.timer = setTimeout(() => { this.timer = null; this.flush(); }, 0);
  }

  dispose(): void {
    this.disposed = true;
    this.scheduled = false;
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
    if (this.channel) {
      this.channel.port1.onmessage = null;
      this.channel.port1.close();
      this.channel.port2.close();
    }
  }

  private flush(): void {
    if (this.disposed || !this.scheduled) return;
    this.scheduled = false;
    this.run();
  }
}
