import type { SyncInputOrigin } from "./sync-input-state";

export type TerminalInputSender = (
  sessionId: string,
  text: string,
  origin: SyncInputOrigin,
  options?: TerminalInputSendOptions,
) => void | Promise<void>;

export type TerminalInputSendOptions = {
  awaitWrite?: boolean;
};

type PendingTerminalInput = {
  sessionId: string;
  text: string;
  origin: SyncInputOrigin;
  options?: TerminalInputSendOptions;
  waiters: Array<{
    resolve: () => void;
    reject: (reason?: unknown) => void;
    propagateErrors: boolean;
  }>;
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

  enqueue(
    sessionId: string,
    text: string,
    origin: SyncInputOrigin,
    options?: TerminalInputSendOptions,
  ): Promise<void> {
    if (!sessionId || !text) return Promise.resolve();
    const completion = new Promise<void>((resolve, reject) => {
      const waiter = {
        resolve,
        reject,
        propagateErrors: Boolean(options?.awaitWrite),
      };
      const tail = this.pending.at(-1);
      if (
        origin === "interactive"
        && tail?.origin === "interactive"
        && tail.sessionId === sessionId
        && !options?.awaitWrite
        && !tail.options?.awaitWrite
      ) {
        tail.text += text;
        tail.waiters.push(waiter);
      } else {
        this.pending.push({ sessionId, text, origin, options, waiters: [waiter] });
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
      result = this.send(item.sessionId, item.text, item.origin, item.options);
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
      for (const waiter of item.waiters) waiter.resolve();
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
      result = this.send(next.sessionId, next.text, next.origin, next.options);
    } catch (error) {
      result = Promise.reject(error);
    }
    void Promise.resolve(result)
      .then(
        () => this.resolveWaiters(next),
        (error) => this.resolveWaiters(next, error, true),
      )
      .finally(() => {
        this.active = false;
        this.drain();
      });
  }

  private resolveWaiters(item: PendingTerminalInput, error?: unknown, failed = false): void {
    for (const waiter of item.waiters) {
      if (failed && waiter.propagateErrors) waiter.reject(error);
      else waiter.resolve();
    }
  }
}

/** Keeps independent session transports from blocking one another. */
export class TerminalInputPumpRegistry {
  private readonly pumps = new Map<string, TerminalInputPump>();

  constructor(private readonly send: TerminalInputSender) {}

  enqueue(
    sessionId: string,
    text: string,
    origin: SyncInputOrigin,
    options?: TerminalInputSendOptions,
  ): Promise<void> {
    if (!sessionId) return Promise.resolve();
    let pump = this.pumps.get(sessionId);
    if (!pump) {
      pump = new TerminalInputPump(this.send);
      this.pumps.set(sessionId, pump);
    }
    return pump.enqueue(sessionId, text, origin, options);
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
      // Do not make a newly reconnected session wait for an IPC request that
      // belonged to the old runtime. The old pump still finishes its request
      // in isolation, while all subsequent keystrokes use a fresh pump.
      const pump = this.pumps.get(sessionId);
      this.pumps.delete(sessionId);
      pump?.reset();
      return;
    }
    const pumps = [...this.pumps.values()];
    this.pumps.clear();
    for (const pump of pumps) pump.reset();
  }
}
