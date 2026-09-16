import { t } from "./i18n";
export type TerminalInputOrder = { streamId: string; sequence: number };
type Binding = { streamId: string };
type Invoke = <T>(command: string, args: Record<string, unknown>) => Promise<T>;
type Stream = { epoch: number; next: number; binding: Promise<Binding> };

/** Per-window streams bind keyboard packets to one native connection instance. */
export class TerminalInputStreams {
  private readonly streams = new Map<string, Stream>();
  private readonly openings = new Map<string, Promise<unknown>>();

  constructor(private readonly invoke: Invoke) {}

  async prepare(sessionId: string, epoch: number): Promise<TerminalInputOrder> {
    let stream = this.streams.get(sessionId);
    if (stream?.epoch !== epoch) {
      this.reset(sessionId);
      // Across a reset, an older bind must not replace a newer native stream
      // if its IPC arrives late. Only the initial handshake is serialized.
      const previous = this.openings.get(sessionId) ?? Promise.resolve();
      const binding = previous.catch(() => {}).then(() => this.invoke<Binding>(
        "begin_terminal_input_stream", { sessionId },
      )).then((binding) => {
        if (!binding || typeof binding.streamId !== "string" || !binding.streamId) {
          throw new Error(t("terminal-input-channel-initialization-failed-reconnect-the-session"));
        }
        return binding;
      });
      stream = { epoch, next: 0, binding };
      this.streams.set(sessionId, stream);
      this.openings.set(sessionId, binding);
      void binding.finally(() => {
        if (this.openings.get(sessionId) === binding) this.openings.delete(sessionId);
      }).catch(() => {});
    }
    const sequence = stream.next++;
    try {
      const binding = await stream.binding;
      if (this.streams.get(sessionId) !== stream) throw new Error(t("terminal-input-cancelled"));
      return { streamId: binding.streamId, sequence };
    } catch (error) {
      if (this.streams.get(sessionId) === stream) this.reset(sessionId);
      throw error;
    }
  }

  reset(sessionId?: string, epoch?: number): void {
    const ids = sessionId ? [sessionId] : [...this.streams.keys()];
    for (const id of ids) {
      const stream = this.streams.get(id);
      if (!stream || (epoch !== undefined && stream.epoch !== epoch)) continue;
      this.streams.delete(id);
      void stream.binding.then((binding) => this.invoke("close_terminal_input_stream", {
        sessionId: id, streamId: binding.streamId,
      })).catch(() => {});
    }
  }

  invalidate(sessionId: string, order: TerminalInputOrder): void {
    const stream = this.streams.get(sessionId);
    if (!stream) return;
    void stream.binding.then((binding) => {
      if (binding.streamId === order.streamId && this.streams.get(sessionId) === stream) this.reset(sessionId);
    }).catch(() => {});
  }
}
