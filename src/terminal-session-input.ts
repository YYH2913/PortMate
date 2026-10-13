import { TerminalInputBoundary, terminalInputCancelled } from "./terminal-input-boundary";
import type { TerminalInputSnapshot } from "./terminal-input-boundary";
import { canPipelineTerminalInput, TerminalInputPumpRegistry } from "./terminal-input-pump";
import type { TerminalInputSendOptions } from "./terminal-input-pump";
import { TerminalInputStreams } from "./terminal-input-stream";
import type { TerminalInputOrder } from "./terminal-input-stream";
import { terminalBinaryStringToBytes } from "./terminal-mouse";
import { formatDirectInput } from "./sync-input-state";
import type { SyncInputOrigin } from "./sync-input-state";
import type { SessionSummary } from "./types";
import { t } from "./i18n";

type Invoke = <T>(command: string, args: Record<string, unknown>) => Promise<T>;

/** One detached session's input lane, bound at admission rather than at dequeue. */
export class TerminalSessionInput {
  private readonly boundary: TerminalInputBoundary;
  private readonly streams: TerminalInputStreams;
  private readonly pumps: TerminalInputPumpRegistry;
  private session: SessionSummary | undefined;

  constructor(
    private readonly invoke: Invoke,
    session?: SessionSummary,
    private readonly callbacks: { onError?: (error: unknown) => void; onSuccess?: () => void } = {},
  ) {
    this.session = session;
    this.boundary = new TerminalInputBoundary(session);
    this.streams = new TerminalInputStreams(invoke);
    this.pumps = new TerminalInputPumpRegistry((sessionId, text, origin, options) => (
      this.send(sessionId, text, origin, options)
    ), { orderedPipeline: true, onReset: (sessionId) => this.streams.reset(sessionId) });
  }

  update(session?: SessionSummary): void {
    this.session = session;
    if (this.boundary.update(session)) this.pumps.reset();
  }

  capture(): TerminalInputSnapshot | null {
    return this.boundary.capture();
  }

  isCurrent(snapshot: TerminalInputSnapshot): boolean {
    return this.boundary.isCurrent(snapshot);
  }

  dispatch(sessionId: string, text: string, origin: SyncInputOrigin, options?: TerminalInputSendOptions): void | Promise<void> {
    let snapshot: TerminalInputSnapshot;
    try {
      snapshot = this.boundary.require(sessionId, options);
    } catch (error) {
      // Ordinary keyboard packets have no acknowledgement consumer. Their
      // stale input is discarded; editors/paste must observe a rejection.
      if (options?.awaitWrite || options?.signal || options?.executeWrite) return Promise.reject(error);
      return;
    }
    if (!options?.binary) text = formatDirectInput(text, this.session?.profile.kind);
    return this.pumps.dispatch(sessionId, text, origin, {
      ...options,
      expectedConnectedSince: snapshot.connectedSince,
      expectedInputEpoch: snapshot.epoch,
    });
  }

  private async send(sessionId: string, text: string, origin: SyncInputOrigin, options?: TerminalInputSendOptions): Promise<void> {
    if (!text) return;
    const snapshot = this.boundary.require(sessionId, options);
    let inputOrder: TerminalInputOrder | undefined;
    try {
      options?.signal?.throwIfAborted();
      if (options?.binary) {
        const bytes = terminalBinaryStringToBytes(text);
        if (!bytes) throw new Error(t("terminal-binary-input-contains-invalid-bytes"));
        await this.invoke("send_bytes", { sessionId, bytes, queued: true, expectedConnectedSince: snapshot.connectedSince });
      } else {
        inputOrder = canPipelineTerminalInput(text, origin, options)
          ? await this.streams.prepare(sessionId, snapshot.epoch)
          : undefined;
        this.boundary.require(sessionId, options);
        options?.signal?.throwIfAborted();
        await this.invoke("send_text", {
          sessionId,
          text,
          interactive: origin === "interactive",
          queued: true,
          expectedConnectedSince: snapshot.connectedSince,
          ...(inputOrder ? { inputOrder } : {}),
          ...(options?.sensitive ? { sensitive: true } : {}),
          ...(options?.awaitWrite ? { awaitWrite: true } : {}),
        });
      }
      if (!this.boundary.isCurrent(snapshot)) throw terminalInputCancelled();
      this.callbacks.onSuccess?.();
    } catch (error) {
      if (inputOrder) this.streams.invalidate(sessionId, inputOrder);
      if (this.boundary.isCurrent(snapshot)) this.callbacks.onError?.(error);
      throw error;
    }
  }
}
