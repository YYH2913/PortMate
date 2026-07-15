import type { TmuxPaneInfo } from "./types";

export interface TmuxWindowGroup {
  target: string;
  session: string;
  windowIndex: number;
  synchronized: boolean;
  panes: TmuxPaneInfo[];
}

export function groupTmuxPanes(panes: readonly TmuxPaneInfo[]): TmuxWindowGroup[] {
  const groups = new Map<string, TmuxWindowGroup>();
  for (const pane of panes) {
    const target = `${pane.session}:${pane.windowIndex}`;
    const existing = groups.get(target);
    if (existing) {
      existing.panes.push(pane);
      existing.synchronized = existing.synchronized && pane.synchronized;
      continue;
    }
    groups.set(target, {
      target,
      session: pane.session,
      windowIndex: pane.windowIndex,
      synchronized: pane.synchronized,
      panes: [pane],
    });
  }
  return [...groups.values()]
    .map((group) => ({
      ...group,
      panes: [...group.panes].sort((left, right) => left.paneIndex - right.paneIndex),
    }))
    .sort((left, right) => (
      left.session.localeCompare(right.session) || left.windowIndex - right.windowIndex
    ));
}
