import { describe, expect, it } from "vitest";
import previousRelease from "../tests/fixtures/release-upgrade/0.1.0/browser-state.json";
import {
  commandHistoryCommands,
  normalizeCommandHistory,
  normalizeCommandHistoryPolicy,
} from "./command-history-state";
import {
  normalizeWorkspaceDockLayout,
  normalizeWorkspaceDockSizes,
  normalizeWorkspacePanelVisibility,
} from "./workspace-panel-state";
import { sanitizeWorkspaceSnapshot, workspacePaneLeaves } from "./workspace-state";

const storage = previousRelease.localStorage;

describe("previous release browser-state upgrade", () => {
  it("preserves the 0.1.0 workspace, panels, docks, sizes, and command history", () => {
    expect(previousRelease.release).toBe("0.1.0");

    const workspace = sanitizeWorkspaceSnapshot(storage["portmate.workspace.v1"]);
    const panes = workspacePaneLeaves(workspace.root);
    expect(workspace).toMatchObject({
      version: 4,
      activePaneId: "release-pane-log",
      activeId: "release-ssh-1",
      root: {
        kind: "split",
        id: "release-split",
        direction: "horizontal",
        ratio: 0.62,
      },
      tabColors: {
        "release-view-router": "#22c55e",
        "release-view-log": "#0ea5e9",
      },
    });
    expect(panes.map((pane) => ({
      id: pane.id,
      activeViewId: pane.activeViewId,
      sessionIds: pane.sessionIds,
      keyMode: pane.views[0]?.keyMode,
    }))).toEqual([
      {
        id: "release-pane-primary",
        activeViewId: "release-view-router",
        sessionIds: ["release-ssh-1"],
        keyMode: "remote",
      },
      {
        id: "release-pane-log",
        activeViewId: "release-view-log",
        sessionIds: ["release-ssh-1"],
        keyMode: "command",
      },
    ]);

    const panels = storage["portmate.workspacePanels.v2"];
    expect(normalizeWorkspacePanelVisibility(panels)).toEqual({
      explorer: true,
      fileManager: true,
      history: true,
      sysmon: true,
      sender: false,
      statusBar: true,
    });
    expect(normalizeWorkspaceDockLayout(panels)).toEqual({
      left: ["explorer", "fileManager"],
      right: ["sysmon", "history"],
      bottom: ["sender"],
      active: {
        left: "fileManager",
        right: "history",
        bottom: "sender",
      },
    });
    expect(normalizeWorkspaceDockSizes(panels)).toEqual({
      left: 360,
      right: 320,
      bottom: 240,
    });

    const history = normalizeCommandHistory(
      storage["portmate.commandHistory"],
      normalizeCommandHistoryPolicy(10_000, 30),
      Date.UTC(2026, 7, 14),
    );
    expect(commandHistoryCommands(history)).toEqual(["show version", "show interfaces"]);
    expect(history.map((entry) => entry.recordedAt)).toEqual([1786388700000, 1786388600000]);
  });
});
