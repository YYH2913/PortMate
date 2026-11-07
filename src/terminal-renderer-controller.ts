import type { TerminalWebglContextInfo } from "./terminal-renderer";

export type TerminalRendererAddon = {
  dispose(): void;
  onContextLoss(listener: () => void): { dispose(): void };
};
type ActiveRenderer<A> = { addon: A; subscription: { dispose(): void } };
type RendererState = {
  renderer: "dom" | "webgl";
  reason: string;
  context?: TerminalWebglContextInfo;
};

/** Own one addon, including failed initialization, stale imports and context loss. */
export class TerminalRendererController<A extends TerminalRendererAddon> {
  private generation = 0;
  private disposed = false;
  private blocked = false;
  private active: ActiveRenderer<A> | null = null;
  private pending: Promise<void> | null = null;

  constructor(private readonly options: {
    load: () => Promise<() => A>;
    activate: (addon: A) => void;
    inspect: () => TerminalWebglContextInfo;
    publish: (state: RendererState) => void;
    disabledReason: string;
  }) {}

  configure(enabled: boolean): Promise<void> {
    if (this.disposed) return Promise.resolve();
    if (!enabled) {
      this.generation++;
      this.pending = null;
      this.blocked = false;
      this.releaseActive();
      this.options.publish({ renderer: "dom", reason: this.options.disabledReason });
      return Promise.resolve();
    }
    if (this.active || this.blocked) return Promise.resolve();
    if (this.pending) return this.pending;
    const generation = ++this.generation;
    const pending = this.start(generation).finally(() => {
      if (this.pending === pending) this.pending = null;
    });
    this.pending = pending;
    return pending;
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.generation++;
    this.pending = null;
    this.releaseActive();
  }

  private isCurrent(generation: number): boolean {
    return !this.disposed && generation === this.generation;
  }

  private async start(generation: number): Promise<void> {
    let candidate: A | null = null;
    let subscription: { dispose(): void } | null = null;
    try {
      const create = await this.options.load();
      if (!this.isCurrent(generation)) return;
      candidate = create();
      const addon = candidate;
      let lost = false;
      subscription = addon.onContextLoss(() => {
        lost = true;
        if (!this.isCurrent(generation) || this.active?.addon !== addon) return;
        this.blocked = true;
        this.releaseActive();
        this.options.publish({ renderer: "dom", reason: "fallback-context-lost" });
      });
      this.options.activate(addon);
      if (!this.isCurrent(generation)) return;
      const context = this.options.inspect();
      if (lost || context.support !== "accelerated") {
        this.blocked = true;
        this.release({ addon, subscription });
        candidate = null;
        subscription = null;
        this.options.publish({ renderer: "dom", reason: lost ? "fallback-context-lost" : `disabled-${context.support}`, context });
        return;
      }
      this.active = { addon, subscription };
      candidate = null;
      subscription = null;
      this.options.publish({ renderer: "webgl", reason: "active", context });
    } catch {
      // loadAddon can fail after partially activating GPU resources. Keep
      // ownership until cleanup even if activation never returned normally.
      if (subscription) { try { subscription.dispose(); } catch { /* best effort */ } }
      subscription = null;
      if (candidate) { try { candidate.dispose(); } catch { /* best effort */ } }
      candidate = null;
      if (this.isCurrent(generation)) {
        this.blocked = true;
        this.releaseActive();
        this.options.publish({ renderer: "dom", reason: "fallback-initialization" });
      }
    } finally {
      if (subscription) { try { subscription.dispose(); } catch { /* best effort */ } }
      if (candidate) { try { candidate.dispose(); } catch { /* best effort */ } }
    }
  }

  private releaseActive(): void {
    const active = this.active;
    this.active = null;
    if (active) this.release(active);
  }

  private release(active: ActiveRenderer<A>): void {
    try { active.subscription.dispose(); } catch { /* One cleanup failure must not leak the addon. */ }
    try { active.addon.dispose(); } catch { /* A lost driver can throw while freeing resources. */ }
  }
}
