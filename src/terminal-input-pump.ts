import type { SyncInputOrigin } from "./sync-input-state";

export type TerminalInputSender = (
  sessionId: string,
  text: string,
  origin: SyncInputOrigin,
) => void | Promise<void>;

type PendingTerminalInput = {
  sessionId: string;
  text: string;
  origin: SyncInputOrigin;
  waiters: Array<() => void>;
};

// Keep a single cancellable IPC boundary. The native queued-input command
// returns immediately, while any burst behind it is merged into one call.
const MAX_FAST_IN_FLIGHT = 1;

/**
 * Starts the first input immediately, then coalesces interactive input while
 * the transport is busy. Atomic input remains an explicit ordering boundary.
 */
export class TerminalInputPump {
  private readonly pending: PendingTerminalInput[] = [];
  private active = false;
  private fastInFlightCount = 0;

  constructor(private readonly send: TerminalInputSender) {}

  enqueue(sessionId: string, text: string, origin: SyncInputOrigin): Promise<void> {
    if (!sessionId || !text) return Promise.resolve();
    const completion = new Promise<void>((resolve) => {
      const tail = this.pending.at(-1);
      if (
        origin === "interactive"
        && tail?.origin === "interactive"
        && tail.sessionId === sessionId
      ) {
        tail.text += text;
        tail.waiters.push(resolve);
      } else {
        this.pending.push({ sessionId, text, origin, waiters: [resolve] });
      }
    });
    this.drain();
    return completion;
  }

  /**
   * Queue high-frequency interactive input without allocating a completion
   * promise for every key. The regular enqueue API remains available for
   * callers that need to await an ordering boundary.
   */
  enqueueFast(sessionId: string, text: string, origin: SyncInputOrigin): void {
    if (!sessionId || !text) return;
    if (origin !== "interactive") {
      void this.enqueue(sessionId, text, origin);
      return;
    }
    if (
      this.active
      || this.fastInFlightCount >= MAX_FAST_IN_FLIGHT
      || this.pending.some((item) => item.origin !== "interactive")
    ) {
      const tail = this.pending.at(-1);
      if (tail?.origin === "interactive" && tail.sessionId === sessionId) {
        tail.text += text;
      } else {
        this.pending.push({ sessionId, text, origin, waiters: [] });
      }
      this.drain();
      return;
    }

    this.launchFast({ sessionId, text, origin, waiters: [] });
  }

  private launchFast(item: PendingTerminalInput): void {
    this.fastInFlightCount += 1;
    let result: void | Promise<void>;
    try {
      result = this.send(item.sessionId, item.text, item.origin);
    } catch {
      result = undefined;
    }
    void Promise.resolve(result)
      .catch(() => {})
      .finally(() => {
        this.fastInFlightCount = Math.max(0, this.fastInFlightCount - 1);
        if (this.fastInFlightCount === 0) this.drain();
      });
  }

  reset(): void {
    for (const item of this.pending) {
      for (const resolve of item.waiters) resolve();
    }
    this.pending.length = 0;
  }

  private drain(): void {
    if (this.active) return;
    if (this.fastInFlightCount > 0) return;
    const next = this.pending.shift();
    if (!next) return;

    if (next.origin === "interactive" && next.waiters.length === 0) {
      this.launchFast(next);
      this.drain();
      return;
    }

    this.active = true;
    let result: void | Promise<void>;
    try {
      result = this.send(next.sessionId, next.text, next.origin);
    } catch {
      result = undefined;
    }
    void Promise.resolve(result)
      .catch(() => {})
      .finally(() => {
        for (const resolve of next.waiters) resolve();
        this.active = false;
        this.drain();
      });
  }
}

/** Keeps independent session transports from blocking one another. */
export class TerminalInputPumpRegistry {
  private readonly pumps = new Map<string, TerminalInputPump>();

  constructor(private readonly send: TerminalInputSender) {}

  enqueue(sessionId: string, text: string, origin: SyncInputOrigin): Promise<void> {
    if (!sessionId) return Promise.resolve();
    let pump = this.pumps.get(sessionId);
    if (!pump) {
      pump = new TerminalInputPump(this.send);
      this.pumps.set(sessionId, pump);
    }
    return pump.enqueue(sessionId, text, origin);
  }

  enqueueFast(sessionId: string, text: string, origin: SyncInputOrigin): void {
    if (!sessionId) return;
    let pump = this.pumps.get(sessionId);
    if (!pump) {
      pump = new TerminalInputPump(this.send);
      this.pumps.set(sessionId, pump);
    }
    pump.enqueueFast(sessionId, text, origin);
  }

  reset(sessionId?: string): void {
    if (sessionId) {
      this.pumps.get(sessionId)?.reset();
      return;
    }
    for (const pump of this.pumps.values()) pump.reset();
  }
}
