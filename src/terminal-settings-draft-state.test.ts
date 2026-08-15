import { describe, expect, it } from "vitest";
import { terminalSettingsDraftHasUnsavedChanges } from "./terminal-settings-draft-state";

describe("terminal settings draft state", () => {
  const saved = {
    prefs: { startupMode: "last", startupSessions: ["edge", ""] },
    syncSettings: { protocols: ["ssh", "serial"], delayMs: 0 },
    workspaceKeymap: { "close-pane": "Alt+KeyX" },
  };

  it("treats structurally equal cloned settings as clean", () => {
    expect(terminalSettingsDraftHasUnsavedChanges(structuredClone(saved), saved)).toBe(false);
    expect(terminalSettingsDraftHasUnsavedChanges({
      workspaceKeymap: { "close-pane": "Alt+KeyX" },
      prefs: { startupSessions: ["edge", ""], startupMode: "last" },
      syncSettings: { delayMs: 0, protocols: ["ssh", "serial"] },
    }, saved)).toBe(false);
  });

  it("detects changes in every settings domain", () => {
    expect(terminalSettingsDraftHasUnsavedChanges({ ...saved, prefs: { ...saved.prefs, startupMode: "none" } }, saved)).toBe(true);
    expect(terminalSettingsDraftHasUnsavedChanges({ ...saved, syncSettings: { ...saved.syncSettings, delayMs: 25 } }, saved)).toBe(true);
    expect(terminalSettingsDraftHasUnsavedChanges({ ...saved, workspaceKeymap: { "close-pane": "" } }, saved)).toBe(true);
    expect(terminalSettingsDraftHasUnsavedChanges({ ...saved, prefs: { ...saved.prefs, startupSessions: ["edge"] } }, saved)).toBe(true);
  });
});
