import type { SessionKind, SessionSummary } from "./types";

export const SESSION_EXPLORER_GROUPING_KEY = "portmate.sessionExplorerGrouping.v1";
const CHANGE_EVENT = "portmate-session-explorer-grouping";
export type SessionExplorerGrouping = "profile" | "type";
const typeOrder: readonly SessionKind[] = ["ssh", "serial", "shell", "telnet", "tcp", "tmux"];
let transient: SessionExplorerGrouping | undefined;

export type SessionExplorerGroup = {
  key: string;
  name: string;
  type: SessionKind | null;
  sessions: SessionSummary[];
};

export function normalizeSessionExplorerGrouping(value: unknown): SessionExplorerGrouping {
  return value === "type" ? "type" : "profile";
}

/** Presentation only: never rewrites the saved Profile.group or connection. */
export function groupExplorerSessions(
  sessions: readonly SessionSummary[],
  grouping: SessionExplorerGrouping,
): SessionExplorerGroup[] {
  const groups = new Map<string, SessionExplorerGroup>();
  for (const session of sessions) {
    const type = grouping === "type" ? session.profile.connection.kind : null;
    const name = type ?? session.profile.group;
    const key = `${grouping}:${name}`;
    let group = groups.get(key);
    if (!group) {
      group = { key, name, type, sessions: [] };
      groups.set(key, group);
    }
    group.sessions.push(session);
  }
  const result = [...groups.values()];
  if (grouping === "type") {
    result.sort((left, right) => typeOrder.indexOf(left.type!) - typeOrder.indexOf(right.type!));
  }
  return result;
}

export function sessionExplorerTypeLabel(kind: SessionKind, serialLabel: string): string {
  switch (kind) {
    case "serial": return serialLabel;
    case "shell": return "Shell";
    case "tmux": return "Tmux";
    case "telnet": return "Telnet";
    case "ssh": return "SSH";
    case "tcp": return "TCP";
  }
}

export function readSessionExplorerGrouping(): SessionExplorerGrouping {
  if (transient !== undefined) return transient;
  try { return normalizeSessionExplorerGrouping(window.localStorage.getItem(SESSION_EXPLORER_GROUPING_KEY)); }
  catch { return "profile"; }
}

export function writeSessionExplorerGrouping(value: SessionExplorerGrouping): void {
  const grouping = normalizeSessionExplorerGrouping(value);
  try {
    window.localStorage.setItem(SESSION_EXPLORER_GROUPING_KEY, grouping);
    transient = undefined;
  } catch {
    // A restricted webview must still allow switching for this app session.
    transient = grouping;
  }
  if (typeof window !== "undefined") window.dispatchEvent(new Event(CHANGE_EVENT));
}

export function subscribeSessionExplorerGrouping(notify: () => void): () => void {
  const remote = (event: StorageEvent) => {
    if (event.key !== SESSION_EXPLORER_GROUPING_KEY && event.key !== null) return;
    try { if (event.storageArea && event.storageArea !== window.localStorage) return; } catch { /* Storage may be denied. */ }
    transient = undefined;
    notify();
  };
  window.addEventListener(CHANGE_EVENT, notify);
  window.addEventListener("storage", remote);
  return () => {
    window.removeEventListener(CHANGE_EVENT, notify);
    window.removeEventListener("storage", remote);
  };
}
