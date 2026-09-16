import { t } from "./i18n";
import type { CustomScript, SaveCustomScriptRequest, HostScriptConfig } from "./types";

export const MAX_CUSTOM_SCRIPTS = 128;
export const MAX_CUSTOM_SCRIPT_NAME_CHARACTERS = 128;
export const MAX_CUSTOM_SCRIPT_DESCRIPTION_CHARACTERS = 1_024;
export const MAX_CUSTOM_SCRIPT_CONTENT_CHARACTERS = 65_536;
export const MAX_CUSTOM_SCRIPT_CONTENT_BYTES = 256 * 1024;

export type CustomScriptDraft = SaveCustomScriptRequest;

export function customScriptDraft(script: CustomScript): CustomScriptDraft {
  return {
    id: script.id,
    name: script.name,
    description: script.description,
    content: script.content,
    host: structuredClone(script.host),
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

export function newCustomScriptDraft(): CustomScriptDraft {
  return {
    id: null,
    name: "",
    description: "",
    content: "",
    host: { language: "python", interpreter: "", workingDirectory: "", timeoutSeconds: 30, allowedClientIds: [], parameters: [] },
    mcpEnabled: false,
    expectedUpdatedAt: null,
  };
}

export function normalizeCustomScriptDraft(draft: CustomScriptDraft): CustomScriptDraft {
  return {
    ...draft,
    name: draft.name.trim(),
    description: draft.description.replace(/[\r\n]/g, " ").trim(),
    // Executable content must never be silently truncated or stripped of NUL.
    // Preserve invalid input so validation can reject it without changing commands.
    content: draft.content.replace(/\r\n?/g, "\n"),
    host: { ...draft.host, allowedClientIds: [...new Set(draft.host.allowedClientIds)] },
  };
}

export function validateCustomScriptDraft(draft: CustomScriptDraft): string | null {
  if (!draft.name.trim()) return t("script-name-is-required");
  if (!draft.content.trim()) return t("script-body-is-required");
  if ([...draft.name].length > MAX_CUSTOM_SCRIPT_NAME_CHARACTERS
    || /[\x00-\x1f\x7f-\x9f]/.test(draft.name)) return t("script-name-is-too-long-or-contains-control-characters");
  if ([...draft.description].length > MAX_CUSTOM_SCRIPT_DESCRIPTION_CHARACTERS
    || /[\x00-\x1f\x7f-\x9f]/.test(draft.description)) return t("script-description-is-too-long-or-contains-control-characters");
  if (draft.content.includes("\0")) return t("script-body-cannot-contain-nul-characters-check-and-save");
  if ([...draft.content].length > MAX_CUSTOM_SCRIPT_CONTENT_CHARACTERS
    || new TextEncoder().encode(draft.content).length > MAX_CUSTOM_SCRIPT_CONTENT_BYTES) {
    return t("script-body-cannot-exceed-characters-or-bytes-content-was", [MAX_CUSTOM_SCRIPT_CONTENT_CHARACTERS, MAX_CUSTOM_SCRIPT_CONTENT_BYTES]);
  }
  if (draft.mcpEnabled && !draft.host.allowedClientIds.length) return t("select-at-least-one-mcp-client");
  return validateHostConfig(draft.host);
}

export function validateHostConfig(host: HostScriptConfig): string | null {
  if (!Number.isInteger(host.timeoutSeconds) || host.timeoutSeconds < 1 || host.timeoutSeconds > 60) return t("timeout-must-be-1-60-seconds");
  for (const path of [host.interpreter, host.workingDirectory]) {
    if (path && !/^(\/|[a-zA-Z]:[\\/]|\\\\)/.test(path)) return t("interpreter-and-working-directory-must-use-absolute-paths");
    if (new TextEncoder().encode(path).length > 4096 || /[\x00-\x1f\x7f-\x9f]/.test(path)) return t("path-is-too-long-or-contains-control-characters");
  }
  if (host.parameters.length > 32) return t("at-most-32-parameters-are-supported");
  const names = new Set<string>();
  for (const parameter of host.parameters) {
    if (!/^[A-Za-z_][A-Za-z0-9_]{0,63}$/.test(parameter.name) || names.has(parameter.name.toLowerCase())) return t("parameter-names-must-be-unique-case-insensitive-contain-only");
    names.add(parameter.name.toLowerCase());
    if (new TextEncoder().encode(parameter.description).length > 1024 || /[\x00-\x1f\x7f-\x9f]/.test(parameter.description)) return t("parameter-description-is-too-long-or-contains-control-characters");
  }
  return null;
}
