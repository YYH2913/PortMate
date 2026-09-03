import type { SyncInputOrigin } from "./sync-input-state";

export type TerminalInputSender = (
  sessionId: string,
  text: string,
  origin: SyncInputOrigin,
  options?: TerminalInputSendOptions,
) => void | Promise<void>;

export type TerminalInputSendOptions = {
  awaitWrite?: boolean;
  /** The text field contains a lossless 0..255 byte string from XTerm. */
  binary?: boolean;
  sensitive?: boolean;
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

// Keep one in-flight request so lifecycle resets can fence every request that
// has not reached the native queue yet. A 1ms adaptive window removes the
// per-key IPC pattern without adding a visible delay to terminal input.
const MAX_FAST_IN_FLIGHT = 1;
const FAST_INITIAL_BATCH_DELAY_MS = 1;
const FAST_FOLLOW_UP_DELAY_MS = 1;

/**
 * Starts the first input immediately, then coalesces interactive input while
 * the transport is busy. Atomic input remains an explicit ordering boundary.
 */
export class TerminalInputPump {
  private readonly pending: PendingTerminalInput[] = [];
  private active = false;
  private fastInFlightCount = 0;
  private fastFlushQueued = false;
  private fastFlushGeneration = 0;
  private fastFlushTimer: ReturnType<typeof setTimeout> | null = null;

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
        && Boolean(options?.binary) === Boolean(tail.options?.binary)
        && Boolean(options?.sensitive) === Boolean(tail.options?.sensitive)
      ) {
        tail.text += text;
        tail.waiters.push(waiter);
      } else {
        this.pending.push({ sessionId, text, origin, options, waiters: [waiter] });
      }
    });
    this.cancelFastDrain();
    this.drain();
    return completion;
  }

  /**
   * Queue high-frequency interactive input without allocating a completion
   * promise for every key. The regular enqueue API remains available for
   * callers that need to await an ordering boundary.
   */
  enqueueFast(
    sessionId: string,
    text: string,
    origin: SyncInputOrigin,
    options?: TerminalInputSendOptions,
  ): void {
    if (!sessionId || !text) return;
    if (origin !== "interactive") {
      void this.enqueue(sessionId, text, origin, options);
      return;
    }
    const tail = this.pending.at(-1);
    if (tail?.origin === "interactive"
      && tail.sessionId === sessionId
      && Boolean(options?.binary) === Boolean(tail.options?.binary)
      && Boolean(options?.sensitive) === Boolean(tail.options?.sensitive)) {
      tail.text += text;
    } else {
      this.pending.push({ sessionId, text, origin, options, waiters: [] });
    }
    this.scheduleFastDrain(FAST_INITIAL_BATCH_DELAY_MS);
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
        if (this.fastInFlightCount === 0) {
          const next = this.pending[0];
          const hasAtomicBoundary = this.pending.some((item) => item.origin !== "interactive");
          if (next?.origin === "interactive" && next.waiters.length === 0 && !hasAtomicBoundary) {
            // A slow IPC response is not a reason to issue one request per
            // key. Give the next event-loop slice a chance to append to the
            // queued payload before crossing another native boundary.
            this.scheduleFastDrain(FAST_FOLLOW_UP_DELAY_MS);
          }
          this.drain();
        }
      });
  }

  reset(): void {
    this.cancelFastDrain();
    for (const item of this.pending) {
      for (const waiter of item.waiters) {
        if (waiter.propagateErrors) {
          waiter.reject(new Error("terminal input was cancelled before the transport write"));
        } else {
          waiter.resolve();
        }
      }
    }
    this.pending.length = 0;
  }

  private scheduleFastDrain(delayMs = 0): void {
    if (this.fastFlushQueued) return;
    this.fastFlushQueued = true;
    const generation = ++this.fastFlushGeneration;
    // Run after the current XTerm callback so a burst emitted in one browser
    // turn crosses the IPC boundary as one request. The short timer also
    // catches adjacent keyboard tasks while keeping isolated keypresses fast.
    const flush = () => {
      if (generation !== this.fastFlushGeneration) return;
      this.fastFlushTimer = null;
      this.fastFlushQueued = false;
      this.drain();
    };
    if (delayMs > 0) this.fastFlushTimer = setTimeout(flush, delayMs);
    else queueMicrotask(flush);
  }

  private cancelFastDrain(): void {
    this.fastFlushGeneration += 1;
    if (this.fastFlushTimer !== null) {
      clearTimeout(this.fastFlushTimer);
      this.fastFlushTimer = null;
    }
    this.fastFlushQueued = false;
  }

  private drain(): void {
    if (this.active) return;
    // Let the current follow-up burst accumulate before crossing another IPC
    // boundary. Atomic enqueue() cancels the microtask and calls drain() directly.
    if (this.fastFlushQueued) return;
    // Fill the bounded fast window before waiting for any IPC response. This
    // path is only used for fire-and-forget printable input; a pending atomic
    // item stops the loop so it cannot be overtaken by later keystrokes.
    while (this.fastInFlightCount < MAX_FAST_IN_FLIGHT) {
      const next = this.pending[0];
      if (!next) return;
      if (next.origin !== "interactive" || next.waiters.length > 0) {
        if (this.fastInFlightCount > 0) return;
        this.pending.shift();
        this.launchOrdered(next);
        return;
      }
      this.pending.shift();
      this.launchFast(next);
    }
  }

  private launchOrdered(next: PendingTerminalInput): void {
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

  enqueueFast(
    sessionId: string,
    text: string,
    origin: SyncInputOrigin,
    options?: TerminalInputSendOptions,
  ): void {
    if (!sessionId) return;
    let pump = this.pumps.get(sessionId);
    if (!pump) {
      pump = new TerminalInputPump(this.send);
      this.pumps.set(sessionId, pump);
    }
    pump.enqueueFast(sessionId, text, origin, options);
  }

  /**
   * Route every input kind through the same per-session ordering lane. Only
   * ordinary printable input uses the fire-and-forget batching path; control
   * keys, paste, commands, and acknowledged writes remain explicit barriers.
   */
  dispatch(
    sessionId: string,
    text: string,
    origin: SyncInputOrigin,
    options?: TerminalInputSendOptions,
  ): void | Promise<void> {
    if (origin === "interactive" && !options?.awaitWrite) {
      this.enqueueFast(sessionId, text, origin, options);
      return;
    }
    return this.enqueue(sessionId, text, origin, options);
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
