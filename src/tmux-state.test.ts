import { describe, expect, it } from "vitest";
import { groupTmuxPanes } from "./tmux-state";
import type { TmuxPaneInfo } from "./types";

describe("tmux state", () => {
  it("groups panes by session window and sorts windows and pane indexes", () => {
    const groups = groupTmuxPanes([
      pane({ session: "beta", windowIndex: 0, paneIndex: 0, paneId: "%4" }),
      pane({ session: "alpha", windowIndex: 2, paneIndex: 1, paneId: "%3" }),
      pane({ session: "alpha", windowIndex: 2, paneIndex: 0, paneId: "%2" }),
      pane({ session: "alpha", windowIndex: 1, paneIndex: 0, paneId: "%1" }),
    ]);

    expect(groups.map((group) => group.target)).toEqual(["alpha:1", "alpha:2", "beta:0"]);
    expect(groups[1].panes.map((item) => item.paneId)).toEqual(["%2", "%3"]);
  });

  it("only reports a window synchronized when every returned pane agrees", () => {
    const groups = groupTmuxPanes([
      pane({ paneId: "%1", paneIndex: 0, synchronized: true }),
      pane({ paneId: "%2", paneIndex: 1, synchronized: false }),
    ]);

    expect(groups).toHaveLength(1);
    expect(groups[0].synchronized).toBe(false);
  });

  it("keeps list-windows metadata while reconciling pane state", () => {
    const groups = groupTmuxPanes(
      [pane({ paneId: "%1", synchronized: true })],
      [{
        session: "alpha",
        windowIndex: 1,
        windowId: "@3",
        name: "metrics",
        panes: 1,
        active: true,
        synchronized: true,
      }],
    );

    expect(groups[0]).toMatchObject({
      target: "alpha:1",
      windowId: "@3",
      name: "metrics",
      active: true,
      synchronized: true,
    });
  });
});

function pane(overrides: Partial<TmuxPaneInfo>): TmuxPaneInfo {
  return {
    session: "alpha",
    windowIndex: 1,
    paneIndex: 0,
    paneId: "%1",
    active: false,
    synchronized: true,
    command: "bash",
    title: "shell",
    ...overrides,
  };
}
