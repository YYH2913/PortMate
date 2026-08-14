import { describe, expect, it } from "vitest";
import {
  customScriptDraft,
  customScriptDraftMatches,
  newCustomScriptDraft,
  normalizeCustomScriptDraft,
  runnableCustomScriptSessions,
  validateCustomScriptDraft,
} from "./custom-script-state";
import type { CustomScript, SessionSummary } from "./types";

describe("custom script state", () => {
  it("creates a session-scoped draft without enabling MCP", () => {
    expect(newCustomScriptDraft("ssh-a")).toMatchObject({
      allowAllSessions: false,
      allowedSessionIds: ["ssh-a"],
      mcpEnabled: false,
    });
  });

  it("normalizes newlines and removes duplicate targets", () => {
    const normalized = normalizeCustomScriptDraft({
      ...newCustomScriptDraft(),
      name: "  inspect  ",
      description: "one\ntwo",
      content: "line 1\r\nline 2\r",
      allowAllSessions: false,
      allowedSessionIds: ["ssh-a", "ssh-a"],
    });
    expect(normalized.name).toBe("inspect");
    expect(normalized.description).toBe("one two");
    expect(normalized.content).toBe("line 1\nline 2\n");
    expect(normalized.allowedSessionIds).toEqual(["ssh-a"]);
  });

  it("requires a body and an explicit target boundary", () => {
    expect(validateCustomScriptDraft(newCustomScriptDraft(""))).toBe("脚本名称不能为空。");
    expect(validateCustomScriptDraft({
      ...newCustomScriptDraft(),
      name: "Inspect",
      allowAllSessions: false,
    })).toBe("脚本正文不能为空。");
    expect(validateCustomScriptDraft({
      ...newCustomScriptDraft(),
      name: "Inspect",
      content: "uptime",
      allowAllSessions: false,
    })).toBe("请选择至少一个会话。");
  });

  it("returns only connected sessions inside the script boundary", () => {
    const sessions = [session("ssh-a", "connected"), session("ssh-b", "disconnected")];
    expect(runnableCustomScriptSessions({ allowAllSessions: true, allowedSessionIds: [] }, sessions).map((item) => item.profile.id)).toEqual(["ssh-a"]);
    expect(runnableCustomScriptSessions({ allowAllSessions: false, allowedSessionIds: ["ssh-b"] }, sessions)).toEqual([]);
  });

  it("copies persisted script target arrays into editable state", () => {
    const script = savedScript();
    const draft = customScriptDraft(script);
    draft.allowedSessionIds.push("ssh-b");
    expect(script.allowedSessionIds).toEqual(["ssh-a"]);
  });

  it("detects unsaved editor changes", () => {
    const script = savedScript();
    const draft = customScriptDraft(script);
    expect(customScriptDraftMatches(draft, script)).toBe(true);
    expect(customScriptDraftMatches({ ...draft, content: `${draft.content}\nwhoami` }, script)).toBe(false);
  });
});

function savedScript(): CustomScript {
  return {
    id: "69c06a07-dc48-4d4e-9498-6f42b6deab21",
    name: "Inspect",
    description: "",
    content: "uptime",
    allowAllSessions: false,
    allowedSessionIds: ["ssh-a"],
    mcpEnabled: true,
    createdAt: "2026-08-14T00:00:00Z",
    updatedAt: "2026-08-14T00:01:00Z",
  };
}

function session(id: string, status: SessionSummary["runtime"]["status"]): SessionSummary {
  return {
    profile: { id, name: id } as SessionSummary["profile"],
    runtime: { sessionId: id, status } as SessionSummary["runtime"],
    logLines: 0,
  };
}
