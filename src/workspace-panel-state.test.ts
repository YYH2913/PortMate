import { describe, expect, it } from "vitest";
import {
  defaultWorkspacePanelVisibility,
  normalizeWorkspacePanelVisibility,
  setWorkspacePanelVisibility,
  toggleWorkspacePanelVisibility,
} from "./workspace-panel-state";

describe("workspace panel state", () => {
  it("uses visible defaults for missing or invalid state", () => {
    expect(normalizeWorkspacePanelVisibility(null)).toEqual(defaultWorkspacePanelVisibility);
    expect(normalizeWorkspacePanelVisibility({ version: 2, panels: { explorer: false } })).toEqual(defaultWorkspacePanelVisibility);
  });

  it("restores v1 snapshots and repairs invalid fields independently", () => {
    expect(normalizeWorkspacePanelVisibility({
      version: 1,
      panels: { explorer: false, fileManager: "no", sender: false, statusBar: false },
    })).toEqual({
      explorer: false,
      fileManager: true,
      sessions: true,
      history: true,
      sender: false,
      statusBar: false,
    });
  });

  it("supports legacy direct objects and immutable updates", () => {
    const initial = normalizeWorkspacePanelVisibility({ history: false });
    const shown = setWorkspacePanelVisibility(initial, "history", true);
    expect(shown).not.toBe(initial);
    expect(shown.history).toBe(true);
    expect(setWorkspacePanelVisibility(shown, "history", true)).toBe(shown);
    expect(toggleWorkspacePanelVisibility(shown, "sessions")).toEqual({ ...shown, sessions: false });
  });
});
