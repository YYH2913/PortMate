import type { SessionSummary } from "./types";

/** IPC creates fresh objects. Share unchanged profiles/runtimes with live views. */
export function reconcileSessionSummaries(current: readonly SessionSummary[], incoming: SessionSummary[]): SessionSummary[] {
  const previous = new Map(current.map(session => [session.profile.id, session]));
  let changed = current.length !== incoming.length;
  const next = incoming.map((session, index) => {
    const old = previous.get(session.profile.id);
    if (!old) { changed = true; return session; }
    const profile = JSON.stringify(old.profile) === JSON.stringify(session.profile) ? old.profile : session.profile;
    const runtime = JSON.stringify(old.runtime) === JSON.stringify(session.runtime) ? old.runtime : session.runtime;
    const value = profile === old.profile && runtime === old.runtime && session.logLines === old.logLines && session.lastLine === old.lastLine
      ? old : { ...session, profile, runtime };
    if (value !== current[index]) changed = true;
    return value;
  });
  return changed ? next : current as SessionSummary[];
}
