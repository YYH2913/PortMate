import type { TmuxPaneInfo, TmuxWindowInfo } from "./types";

export const MAX_STOPPED_TMUX_CONTROL_RUNTIME_IDS = 256;

export interface TmuxWindowGroup {
  target: string;
  session: string;
  windowIndex: number;
  windowId: string;
  name: string;
  active: boolean;
  synchronized: boolean;
  panes: TmuxPaneInfo[];
}

export function rememberStoppedTmuxControlRuntimeId(
  runtimeIds: Set<string>,
  runtimeId: string,
  limit = MAX_STOPPED_TMUX_CONTROL_RUNTIME_IDS,
) {
  if (!runtimeId) return;
  runtimeIds.delete(runtimeId);
  runtimeIds.add(runtimeId);
  while (runtimeIds.size > Math.max(1, Math.trunc(limit))) {
    const oldest = runtimeIds.values().next().value;
    if (oldest === undefined) break;
    runtimeIds.delete(oldest);
  }
}

export function groupTmuxPanes(
  panes: readonly TmuxPaneInfo[],
  windows: readonly TmuxWindowInfo[] = [],
): TmuxWindowGroup[] {
  const groups = new Map<string, TmuxWindowGroup>();
  for (const window of windows) {
    const target = `${window.session}:${window.windowIndex}`;
    groups.set(target, {
      target,
      session: window.session,
      windowIndex: window.windowIndex,
      windowId: window.windowId,
      name: window.name,
      active: window.active,
      synchronized: window.synchronized,
      panes: [],
    });
  }
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
      windowId: "",
      name: "",
      active: pane.active,
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
