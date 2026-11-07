import type { SessionEvent } from "./types";

/** Apply a shard-metadata burst with one scan/copy per retained session log. */
export function reconcileSessionEventUpdates(
  current: Record<string, SessionEvent[]>,
  updates: readonly SessionEvent[],
): { logs: Record<string, SessionEvent[]>; changedSessions: string[] } {
  const bySession = new Map<string, Map<string, SessionEvent>>();
  for (const event of updates) {
    let events = bySession.get(event.sessionId);
    if (!events) bySession.set(event.sessionId, events = new Map());
    events.set(event.id, event);
  }
  let logs = current;
  const changedSessions: string[] = [];
  for (const [sessionId, updatesById] of bySession) {
    const existing = current[sessionId];
    if (!Array.isArray(existing)) continue;
    let changed: SessionEvent[] | null = null;
    for (let index = 0; index < existing.length; index++) {
      const event = existing[index];
      const updated = updatesById.get(event.id);
      if (!updated || updated === event) continue;
      changed ??= existing.slice();
      changed[index] = updated;
    }
    if (!changed) continue;
    if (logs === current) logs = { ...current };
    Object.defineProperty(logs, sessionId, { value: changed, writable: true, configurable: true, enumerable: true });
    changedSessions.push(sessionId);
  }
  return { logs, changedSessions };
}
