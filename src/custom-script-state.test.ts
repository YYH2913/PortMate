import { describe, expect, it } from "vitest";
import {
  customScriptDraft,
  customScriptDraftMatches,
  MAX_CUSTOM_SCRIPT_CONTENT_CHARACTERS,
  newCustomScriptDraft,
  normalizeCustomScriptDraft,
  validateCustomScriptDraft,
} from "./custom-script-state";
import type { CustomScript } from "./types";

describe("custom script state", () => {
  it("creates a host Python draft without enabling MCP", () => {
    expect(newCustomScriptDraft()).toMatchObject({
      host: { language: "python", allowedClientIds: [] },
      mcpEnabled: false,
    });
  });

  it("normalizes newlines and removes duplicate targets", () => {
    const normalized = normalizeCustomScriptDraft({
      ...newCustomScriptDraft(),
      name: "  inspect  ",
      description: "one\ntwo",
      content: "line 1\r\nline 2\r",
      host: { ...newCustomScriptDraft().host, allowedClientIds: ["client-a", "client-a"] },
    });
    expect(normalized.name).toBe("inspect");
    expect(normalized.description).toBe("one two");
    expect(normalized.content).toBe("line 1\nline 2\n");
    expect(normalized.host.allowedClientIds).toEqual(["client-a"]);
  });

  it("requires a body and an explicit target boundary", () => {
    expect(validateCustomScriptDraft(newCustomScriptDraft())).toBe("脚本名称不能为空。");
    expect(validateCustomScriptDraft({
      ...newCustomScriptDraft(),
      name: "Inspect",
      mcpEnabled: true,
    })).toBe("脚本正文不能为空。");
    expect(validateCustomScriptDraft({
      ...newCustomScriptDraft(),
      name: "Inspect",
      content: "uptime",
      mcpEnabled: true,
    })).toBe("请选择至少一个 MCP 客户端。");
  });


  it("copies persisted script target arrays into editable state", () => {
    const script = savedScript();
    const draft = customScriptDraft(script);
    draft.host.allowedClientIds.push("client-b");
    expect(script.host.allowedClientIds).toEqual(["client-a"]);
  });

  it("detects unsaved editor changes", () => {
    const script = savedScript();
    const draft = customScriptDraft(script);
    expect(customScriptDraftMatches(draft, script)).toBe(true);
    expect(customScriptDraftMatches({ ...draft, content: `${draft.content}\nwhoami` }, script)).toBe(false);
  });

  it("rejects oversized or NUL-containing bodies without silently rewriting commands", () => {
    for (const content of ["x".repeat(MAX_CUSTOM_SCRIPT_CONTENT_CHARACTERS + 1), "echo a\0b"]) {
      const draft = { ...customScriptDraft(savedScript()), content };
      const normalized = normalizeCustomScriptDraft(draft);
      expect(normalized.content).toBe(content);
      expect(validateCustomScriptDraft(normalized)).not.toBeNull();
      expect(customScriptDraftMatches(draft, savedScript())).toBe(false);
    }
  });

  it("does not hide an appended suffix beyond a saved maximum-length body", () => {
    const script = { ...savedScript(), content: "x".repeat(MAX_CUSTOM_SCRIPT_CONTENT_CHARACTERS) };
    const draft = { ...customScriptDraft(script), content: `${script.content}y` };
    expect(customScriptDraftMatches(draft, script)).toBe(false);
  });

  it("counts supplementary Unicode characters as characters rather than UTF-16 units", () => {
    const draft = { ...customScriptDraft(savedScript()), content: "😀".repeat(MAX_CUSTOM_SCRIPT_CONTENT_CHARACTERS) };
    expect(validateCustomScriptDraft(draft)).toBeNull();
    expect(normalizeCustomScriptDraft(draft).content).toBe(draft.content);
  });
});

function savedScript(): CustomScript {
  return {
    id: "69c06a07-dc48-4d4e-9498-6f42b6deab21",
    name: "Inspect",
    description: "",
    content: "uptime",
    host: { ...newCustomScriptDraft().host, allowedClientIds: ["client-a"] },
    mcpEnabled: true,
    createdAt: "2026-08-14T00:00:00Z",
    updatedAt: "2026-08-14T00:01:00Z",
  };
}
