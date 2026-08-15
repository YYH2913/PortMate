import { AsyncOperationQueue } from "./async-operation-queue";
import type { SessionKind } from "./types";

export type SyncNewlineMode = "protocol" | "preserve" | "lf" | "crlf";
export type SyncInputOrigin = "interactive" | "atomic" | "command";

export interface SyncInputSettings {
  protocols: SessionKind[];
  newlineMode: SyncNewlineMode;
  delayMs: number;
  prefix: string;
  suffix: string;
}

export interface SyncInputCandidate {
  id: string;
  kind: SessionKind;
  connected: boolean;
}

export interface SyncInputBatch {
  sourceId: string;
  text: string;
  broadcastEnabled: boolean;
  applyAffixes: boolean;
  settings: SyncInputSettings;
  candidates: SyncInputCandidate[];
}

export interface SyncInputDispatchResult {
  succeeded: string[];
  failed: string[];
  skipped: string[];
}

export const allSyncProtocols: SessionKind[] = ["ssh", "tmux", "serial", "shell", "telnet", "tcp"];

export const defaultSyncInputSettings: SyncInputSettings = {
  protocols: [...allSyncProtocols],
  newlineMode: "protocol",
  delayMs: 0,
  prefix: "",
  suffix: "",
};

export function normalizeSyncInputSettings(value: unknown): SyncInputSettings {
  if (!value || typeof value !== "object") return { ...defaultSyncInputSettings, protocols: [...allSyncProtocols] };
  const source = value as Partial<SyncInputSettings>;
  const protocols = Array.isArray(source.protocols)
    ? source.protocols.filter((protocol, index): protocol is SessionKind => (
      allSyncProtocols.includes(protocol as SessionKind) && source.protocols?.indexOf(protocol) === index
    ))
    : [...allSyncProtocols];
  const newlineMode = source.newlineMode === "preserve" || source.newlineMode === "lf" || source.newlineMode === "crlf"
    ? source.newlineMode
    : "protocol";
  const delayMs = Number.isFinite(source.delayMs)
    ? Math.min(5000, Math.max(0, Math.trunc(source.delayMs ?? 0)))
    : 0;
  return {
    protocols,
    newlineMode,
    delayMs,
    prefix: typeof source.prefix === "string" ? source.prefix.slice(0, 1024) : "",
    suffix: typeof source.suffix === "string" ? source.suffix.slice(0, 1024) : "",
  };
}

export function resolveSyncInputTargets(
  sourceId: string,
  candidates: SyncInputCandidate[],
  settings: SyncInputSettings,
): string[] {
  if (!sourceId) return [];
  const source = candidates.find((candidate) => candidate.id === sourceId);
  if (!source) return [sourceId];
  const targets = [source.id];
  for (const candidate of candidates) {
    if (
      candidate.id !== sourceId
      && candidate.connected
      && settings.protocols.includes(candidate.kind)
      && !targets.includes(candidate.id)
    ) {
      targets.push(candidate.id);
    }
  }
  return targets;
}

export function formatSyncInput(
  text: string,
  settings: SyncInputSettings,
  targetKind?: SessionKind,
  applyAffixes = true,
): string {
  let payload = applyAffixes ? `${settings.prefix}${text}${settings.suffix}` : text;
  if (settings.newlineMode === "protocol" && targetKind === "telnet") {
    payload = payload.replace(/\r(?!\n)/g, "\r\n");
  } else if (settings.newlineMode === "lf" || settings.newlineMode === "crlf") {
    const newline = settings.newlineMode === "crlf" ? "\r\n" : "\n";
    payload = payload.replace(/\r\n|\r|\n/g, newline);
  }
  return payload;
}

export class SyncInputDispatcher {
  private readonly operationQueue = new AsyncOperationQueue();
  private broadcastGeneration = 0;
  private readonly cancellationWaiters = new Set<() => void>();

  constructor(private readonly wait: (milliseconds: number) => Promise<void> = defaultWait) {}

  cancelBroadcasts(): void {
    this.broadcastGeneration += 1;
    for (const cancel of this.cancellationWaiters) cancel();
    this.cancellationWaiters.clear();
  }

  enqueue(
    batch: SyncInputBatch,
    send: (sessionId: string, text: string) => Promise<void>,
    isBroadcastEnabled: () => boolean,
  ): Promise<SyncInputDispatchResult> {
    const generation = this.broadcastGeneration;
    return this.enqueueOperation(() => this.dispatch(batch, send, isBroadcastEnabled, generation));
  }

  enqueueOperation<Result>(operation: () => Promise<Result>): Promise<Result> {
    return this.operationQueue.enqueue(operation);
  }

  private async dispatch(
    batch: SyncInputBatch,
    send: (sessionId: string, text: string) => Promise<void>,
    isBroadcastEnabled: () => boolean,
    generation: number,
  ): Promise<SyncInputDispatchResult> {
    const targets = batch.broadcastEnabled
      ? resolveSyncInputTargets(batch.sourceId, batch.candidates, batch.settings)
      : [batch.sourceId];
    const result: SyncInputDispatchResult = { succeeded: [], failed: [], skipped: [] };
    for (let index = 0; index < targets.length; index += 1) {
      const sessionId = targets[index];
      if (index > 0 && (!isBroadcastEnabled() || generation !== this.broadcastGeneration)) {
        result.skipped.push(...targets.slice(index));
        break;
      }
      const candidate = batch.candidates.find((item) => item.id === sessionId);
      const payload = batch.broadcastEnabled
        ? formatSyncInput(batch.text, batch.settings, candidate?.kind, batch.applyAffixes)
        : formatDirectInput(batch.text, candidate?.kind);
      try {
        await send(sessionId, payload);
        result.succeeded.push(sessionId);
      } catch {
        result.failed.push(sessionId);
      }
      if (
        index + 1 < targets.length
        && batch.settings.delayMs > 0
        && isBroadcastEnabled()
        && generation === this.broadcastGeneration
      ) {
        await this.waitForBroadcastDelay(batch.settings.delayMs);
      }
    }
    return result;
  }

  private async waitForBroadcastDelay(milliseconds: number): Promise<void> {
    let cancel = () => {};
    const cancelled = new Promise<void>((resolve) => {
      cancel = resolve;
      this.cancellationWaiters.add(cancel);
    });
    try {
      await Promise.race([this.wait(milliseconds), cancelled]);
    } finally {
      this.cancellationWaiters.delete(cancel);
    }
  }
}

function formatDirectInput(text: string, targetKind?: SessionKind): string {
  return targetKind === "telnet" ? text.replace(/\r(?!\n)/g, "\r\n") : text;
}

function defaultWait(milliseconds: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, milliseconds));
}
