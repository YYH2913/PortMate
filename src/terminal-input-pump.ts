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
  /** Cancellable acknowledged sender-panel operation in this session's lane. */
  signal?: AbortSignal;
  executeWrite?: () => Promise<void>;
};

type PendingTerminalInput = {
  sessionId: string;
  text: string;
  origin: SyncInputOrigin;
  options?: TerminalInputSendOptions;
  detachAbort?: () => void;
  waiters: Array<{
    resolve: () => void;
    reject: (reason?: unknown) => void;
    propagateErrors: boolean;
  }>;
};

type OrderedFlight = { settled: boolean; item: PendingTerminalInput; failed: boolean; error?: unknown };

// Legacy senders remain single-flight. Desktop senders opt into the bounded
// window only with native sequence ordering and a connection-bound stream.
// Microtasks combine same-turn bursts without adding a timer delay.
const MAX_FAST_IN_FLIGHT = 1;
const MAX_ORDERED_IN_FLIGHT = 8;
const MAX_PIPELINED_TEXT_CHARACTERS = 4096;

export function canPipelineTerminalInput(text: string, origin: SyncInputOrigin, options?: TerminalInputSendOptions) {
  return origin !== "command" && !options?.awaitWrite && !options?.binary
    && !options?.signal && !options?.executeWrite
    && text.length <= MAX_PIPELINED_TEXT_CHARACTERS;
}

type TerminalInputPumpOptions = {
  /** Requires a sender with native sequence ordering and runtime fencing. */
  orderedPipeline?: boolean;
  onReset?: (sessionId?: string) => void;
};

/**
 * Coalesces interactive input within one browser turn and while IPC is busy.
 * Control frames remain intact. Ordered desktop streams can pipeline them;
 * commands, binary frames and acknowledged writes remain IPC barriers.
 */
export class TerminalInputPump {
  private readonly pending: PendingTerminalInput[] = [];
  private active = false;
  private fastInFlightCount = 0;
  private fastFlushQueued = false;
  private fastFlushGeneration = 0;
  private readonly orderedFlights: OrderedFlight[] = [];

  constructor(private readonly send: TerminalInputSender, private readonly config: TerminalInputPumpOptions = {}) {}

  enqueue(
    sessionId: string,
    text: string,
    origin: SyncInputOrigin,
    options?: TerminalInputSendOptions,
  ): Promise<void> {
    if (options?.signal?.aborted) return Promise.reject(new DOMException("发送已取消", "AbortError"));
    if (!sessionId || !text) return Promise.resolve();
    const completion = new Promise<void>((resolve, reject) => {
      const waiter = {
        resolve,
        reject,
        propagateErrors: Boolean(options?.awaitWrite || options?.signal || options?.executeWrite),
      };
      const tail = this.pending.at(-1);
      if (
        origin === "interactive"
        && tail?.origin === "interactive"
        && tail.sessionId === sessionId
        && !options?.awaitWrite
        && !tail.options?.awaitWrite
        && !options?.signal && !tail.options?.signal
        && !options?.executeWrite && !tail.options?.executeWrite
        && Boolean(options?.binary) === Boolean(tail.options?.binary)
        && Boolean(options?.sensitive) === Boolean(tail.options?.sensitive)
        && (!this.config.orderedPipeline || tail.text.length + text.length <= MAX_PIPELINED_TEXT_CHARACTERS)
      ) {
        tail.text += text;
        tail.waiters.push(waiter);
      } else {
        const item: PendingTerminalInput = { sessionId, text, origin, options, waiters: [waiter] };
        this.pending.push(item);
        if (options?.signal) {
          const signal = options.signal;
          const abort = () => {
            const index = this.pending.indexOf(item);
            if (index < 0) return;
            this.pending.splice(index, 1);
            item.detachAbort?.();
            this.resolveWaiters(item, new DOMException("发送已取消", "AbortError"), true);
            this.drain();
          };
          item.detachAbort = () => signal.removeEventListener("abort", abort);
          signal.addEventListener("abort", abort, { once: true });
        }
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
      && !tail.options?.awaitWrite && !options?.awaitWrite
      && !tail.options?.signal && !options?.signal
      && !tail.options?.executeWrite && !options?.executeWrite
      && Boolean(options?.binary) === Boolean(tail.options?.binary)
      && Boolean(options?.sensitive) === Boolean(tail.options?.sensitive)
      && (!this.config.orderedPipeline || tail.text.length + text.length <= MAX_PIPELINED_TEXT_CHARACTERS)) {
      tail.text += text;
    } else {
      this.pending.push({ sessionId, text, origin, options, waiters: [] });
    }
    this.scheduleFastDrain();
  }

  private launchFast(item: PendingTerminalInput): void {
    this.fastInFlightCount += 1;
    const flight: OrderedFlight = { settled: false, item, failed: false };
    if (this.config.orderedPipeline) this.orderedFlights.push(flight);
    let result: void | Promise<void>;
    try {
      result = this.send(item.sessionId, item.text, item.origin, item.options);
    } catch (error) {
      result = Promise.reject(error);
    }
    void Promise.resolve(result)
      .then(
        () => { if (!this.config.orderedPipeline) this.resolveWaiters(item); },
        (error) => {
          flight.failed = true;
          flight.error = error;
          if (!this.config.orderedPipeline) this.resolveWaiters(item, error, true);
        },
      )
      .finally(() => {
        if (this.config.orderedPipeline) {
          // Slide credit only over a contiguous acknowledged prefix. A late
          // first request cannot let later replies grow native reordering
          // buffers indefinitely, even when the remaining calls finish fast.
          flight.settled = true;
          while (this.orderedFlights[0]?.settled) {
            const completed = this.orderedFlights.shift()!;
            this.resolveWaiters(completed.item, completed.error, completed.failed);
            this.fastInFlightCount -= 1;
          }
          this.drain();
          return;
        }
        this.fastInFlightCount = Math.max(0, this.fastInFlightCount - 1);
        if (this.fastInFlightCount === 0) {
          const next = this.pending[0];
          const hasAtomicBoundary = this.pending.some((item) => item.origin !== "interactive");
          if (next?.origin === "interactive" && next.waiters.length === 0 && !hasAtomicBoundary) {
            // A slow IPC response is not a reason to issue one request per
            // key. Give the next event-loop slice a chance to append to the
            // queued payload before crossing another native boundary.
            this.scheduleFastDrain();
          }
          this.drain();
        }
      });
  }

  reset(): void {
    this.cancelFastDrain();
    for (const item of this.pending) {
      item.detachAbort?.();
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

  private scheduleFastDrain(): void {
    if (this.fastFlushQueued) return;
    this.fastFlushQueued = true;
    const generation = ++this.fastFlushGeneration;
    // Run after the current XTerm callback so a burst emitted in one browser
    // turn crosses the IPC boundary as one request.
    queueMicrotask(() => {
      if (generation !== this.fastFlushGeneration) return;
      this.fastFlushQueued = false;
      this.drain();
    });
  }

  private cancelFastDrain(): void {
    this.fastFlushGeneration += 1;
    this.fastFlushQueued = false;
  }

  private drain(): void {
    if (this.active) return;
    // Let the current follow-up burst accumulate before crossing another IPC
    // boundary. Atomic enqueue() cancels the microtask and calls drain() directly.
    if (this.fastFlushQueued) return;
    // Fill the admission window without waiting for each IPC round trip.
    // Explicit barriers cannot be overtaken by later keyboard packets.
    while (this.fastInFlightCount < (this.config.orderedPipeline ? MAX_ORDERED_IN_FLIGHT : MAX_FAST_IN_FLIGHT)) {
      const next = this.pending[0];
      if (!next) return;
      const pipeline = this.config.orderedPipeline
        ? canPipelineTerminalInput(next.text, next.origin, next.options)
        : next.origin === "interactive" && next.waiters.length === 0 && !next.options?.executeWrite;
      if (!pipeline) {
        if (this.fastInFlightCount > 0) return;
        this.pending.shift();
        next.detachAbort?.();
        this.launchOrdered(next);
        return;
      }
      this.pending.shift();
      next.detachAbort?.();
      this.launchFast(next);
    }
  }

  private launchOrdered(next: PendingTerminalInput): void {
    this.active = true;
    let result: void | Promise<void>;
    try {
      next.options?.signal?.throwIfAborted();
      result = next.options?.executeWrite ? next.options.executeWrite()
        : this.send(next.sessionId, next.text, next.origin, next.options);
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

  constructor(private readonly send: TerminalInputSender, private readonly config: TerminalInputPumpOptions = {}) {}

  enqueue(
    sessionId: string,
    text: string,
    origin: SyncInputOrigin,
    options?: TerminalInputSendOptions,
  ): Promise<void> {
    if (!sessionId) return Promise.resolve();
    let pump = this.pumps.get(sessionId);
    if (!pump) {
      pump = new TerminalInputPump(this.send, this.config);
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
      pump = new TerminalInputPump(this.send, this.config);
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
    if (origin === "interactive" && !options?.awaitWrite && !options?.signal && !options?.executeWrite) {
      this.enqueueFast(sessionId, text, origin, options);
      return;
    }
    return this.enqueue(sessionId, text, origin, options);
  }

  reset(sessionId?: string): void {
    this.config.onReset?.(sessionId);
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
