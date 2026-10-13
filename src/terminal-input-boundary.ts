import { t } from "./i18n";
import type { TerminalInputSendOptions } from "./terminal-input-pump";
import type { SessionSummary } from "./types";

export type TerminalInputSnapshot = {
  sessionId: string;
  connectedSince: string | null;
  epoch: number;
};

/** Fences asynchronous input even when a reconnect reuses a wall-clock timestamp. */
export class TerminalInputBoundary {
  private key = "";
  private epoch = 0;
  private session: SessionSummary | undefined;

  constructor(session?: SessionSummary) {
    this.update(session);
  }

  update(session?: SessionSummary): boolean {
    const key = JSON.stringify([
      session?.profile.id, session?.profile.kind,
      session?.runtime.status, session?.runtime.connectedSince ?? null,
    ]);
    this.session = session;
    if (key === this.key) return false;
    this.key = key;
    this.epoch += 1;
    return true;
  }

  capture(sessionId = this.session?.profile.id): TerminalInputSnapshot | null {
    if (!sessionId || this.session?.profile.id !== sessionId || this.session.runtime.status !== "connected") return null;
    return { sessionId, connectedSince: this.session.runtime.connectedSince ?? null, epoch: this.epoch };
  }

  isCurrent(snapshot: TerminalInputSnapshot): boolean {
    const current = this.capture(snapshot.sessionId);
    return current !== null && current.epoch === snapshot.epoch && current.connectedSince === snapshot.connectedSince;
  }

  require(sessionId: string, options?: Pick<TerminalInputSendOptions, "expectedConnectedSince" | "expectedInputEpoch">): TerminalInputSnapshot {
    const snapshot = this.capture(sessionId);
    if (!snapshot
      || (options?.expectedConnectedSince !== undefined && options.expectedConnectedSince !== snapshot.connectedSince)
      || (options?.expectedInputEpoch !== undefined && options.expectedInputEpoch !== snapshot.epoch)) {
      throw terminalInputCancelled();
    }
    return snapshot;
  }
}

export function terminalInputCancelled(): DOMException {
  return new DOMException(t("terminal-input-cancelled"), "AbortError");
}

/** Capture before readText, then validate again before xterm emits any input/history. */
export async function pasteTerminalClipboard(
  boundary: TerminalInputBoundary,
  readText: () => Promise<string>,
  paste: (text: string, snapshot: TerminalInputSnapshot) => void,
  canPaste: () => boolean,
): Promise<boolean> {
  const snapshot = boundary.capture();
  if (!snapshot || !canPaste()) return false;
  const text = await readText();
  if (!text || !boundary.isCurrent(snapshot) || !canPaste()) return false;
  paste(text, snapshot);
  return true;
}

/** Start the write synchronously to preserve input order; commit only its current acknowledgement. */
export async function commitTerminalInputAfterWrite(
  boundary: TerminalInputBoundary,
  snapshot: TerminalInputSnapshot,
  write: () => void | Promise<void>,
  commit: () => void,
  invalidate: () => void,
): Promise<boolean> {
  if (!boundary.isCurrent(snapshot)) return false;
  try {
    await write();
    if (!boundary.isCurrent(snapshot)) return false;
    commit();
    return true;
  } catch {
    if (boundary.isCurrent(snapshot)) invalidate();
    return false;
  }
}
