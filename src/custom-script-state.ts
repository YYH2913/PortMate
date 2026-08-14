import type { CustomScript, SaveCustomScriptRequest, SessionSummary } from "./types";

export const MAX_CUSTOM_SCRIPTS = 128;
export const MAX_CUSTOM_SCRIPT_NAME_CHARACTERS = 128;
export const MAX_CUSTOM_SCRIPT_DESCRIPTION_CHARACTERS = 1_024;
export const MAX_CUSTOM_SCRIPT_CONTENT_CHARACTERS = 65_536;

export type CustomScriptDraft = SaveCustomScriptRequest;

export function customScriptDraft(script: CustomScript): CustomScriptDraft {
  return {
    id: script.id,
    name: script.name,
    description: script.description,
    content: script.content,
    allowAllSessions: script.allowAllSessions,
    allowedSessionIds: [...script.allowedSessionIds],
    mcpEnabled: script.mcpEnabled,
    expectedUpdatedAt: script.updatedAt,
  };
}

export function customScriptDraftMatches(
  draft: CustomScriptDraft,
  script: CustomScript,
): boolean {
  return JSON.stringify(normalizeCustomScriptDraft(draft)) === JSON.stringify(customScriptDraft(script));
}

export function newCustomScriptDraft(activeSessionId = ""): CustomScriptDraft {
  return {
    id: null,
    name: "",
    description: "",
    content: "",
    allowAllSessions: !activeSessionId,
    allowedSessionIds: activeSessionId ? [activeSessionId] : [],
    mcpEnabled: false,
    expectedUpdatedAt: null,
  };
}

export function normalizeCustomScriptDraft(draft: CustomScriptDraft): CustomScriptDraft {
  return {
    ...draft,
    name: limitUnicode(draft.name.replace(/\0/g, "").trim(), MAX_CUSTOM_SCRIPT_NAME_CHARACTERS),
    description: limitUnicode(draft.description.replace(/[\0\r\n]/g, " ").trim(), MAX_CUSTOM_SCRIPT_DESCRIPTION_CHARACTERS),
    content: limitUnicode(draft.content.replace(/\r\n?/g, "\n").replace(/\0/g, ""), MAX_CUSTOM_SCRIPT_CONTENT_CHARACTERS),
    allowedSessionIds: draft.allowAllSessions ? [] : [...new Set(draft.allowedSessionIds)],
  };
}

export function validateCustomScriptDraft(draft: CustomScriptDraft): string | null {
  if (!draft.name.trim()) return "脚本名称不能为空。";
  if (!draft.content.trim()) return "脚本正文不能为空。";
  if (!draft.allowAllSessions && !draft.allowedSessionIds.length) return "请选择至少一个会话。";
  return null;
}

export function scriptAllowsSession(script: Pick<CustomScriptDraft, "allowAllSessions" | "allowedSessionIds">, sessionId: string): boolean {
  return script.allowAllSessions || script.allowedSessionIds.includes(sessionId);
}

export function runnableCustomScriptSessions(
  draft: Pick<CustomScriptDraft, "allowAllSessions" | "allowedSessionIds">,
  sessions: readonly SessionSummary[],
): SessionSummary[] {
  return sessions.filter((session) => (
    session.runtime.status === "connected" && scriptAllowsSession(draft, session.profile.id)
  ));
}

function limitUnicode(value: string, maximum: number): string {
  return [...value].slice(0, maximum).join("");
}
