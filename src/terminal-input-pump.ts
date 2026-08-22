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
};

/**
 * Starts the first input immediately, then coalesces interactive input while
 * the transport is busy. Atomic input remains an explicit ordering boundary.
 */
export class TerminalInputPump {
  private readonly pending: PendingTerminalInput[] = [];
  private active = false;

  constructor(private readonly send: TerminalInputSender) {}

  enqueue(sessionId: string, text: string, origin: SyncInputOrigin): void {
    if (!sessionId || !text) return;
    const tail = this.pending.at(-1);
    if (
      origin === "interactive"
      && tail?.origin === "interactive"
      && tail.sessionId === sessionId
    ) {
      tail.text += text;
    } else {
      this.pending.push({ sessionId, text, origin });
    }
    this.drain();
  }

  reset(): void {
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
        this.active = false;
        this.drain();
      });
  }
}
