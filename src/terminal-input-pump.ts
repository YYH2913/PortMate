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

/**
 * Starts the first input immediately, then coalesces interactive input while
 * the transport is busy. Atomic input remains an explicit ordering boundary.
 */
export class TerminalInputPump {
  private readonly pending: PendingTerminalInput[] = [];
  private active = false;

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

  reset(): void {
    for (const item of this.pending) {
      for (const resolve of item.waiters) resolve();
    }
    this.pending.length = 0;
  }

  private drain(): void {
    if (this.active) return;
    const next = this.pending.shift();
    if (!next) return;

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

  reset(sessionId?: string): void {
    if (sessionId) {
      this.pumps.get(sessionId)?.reset();
      return;
    }
    for (const pump of this.pumps.values()) pump.reset();
  }
}
