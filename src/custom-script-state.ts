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
  if (!draft.name.trim()) return "脚本名称不能为空。";
  if (!draft.content.trim()) return "脚本正文不能为空。";
  if ([...draft.name].length > MAX_CUSTOM_SCRIPT_NAME_CHARACTERS
    || /[\x00-\x1f\x7f-\x9f]/.test(draft.name)) return "脚本名称过长或包含控制字符。";
  if ([...draft.description].length > MAX_CUSTOM_SCRIPT_DESCRIPTION_CHARACTERS
    || /[\x00-\x1f\x7f-\x9f]/.test(draft.description)) return "脚本说明过长或包含控制字符。";
  if (draft.content.includes("\0")) return "脚本正文不能包含 NUL 字符，请检查后重新保存。";
  if ([...draft.content].length > MAX_CUSTOM_SCRIPT_CONTENT_CHARACTERS
    || new TextEncoder().encode(draft.content).length > MAX_CUSTOM_SCRIPT_CONTENT_BYTES) {
    return `脚本正文不能超过 ${MAX_CUSTOM_SCRIPT_CONTENT_CHARACTERS} 个字符或 ${MAX_CUSTOM_SCRIPT_CONTENT_BYTES} 字节；内容未被截断。`;
  }
  if (draft.mcpEnabled && !draft.host.allowedClientIds.length) return "请选择至少一个 MCP 客户端。";
  return validateHostConfig(draft.host);
}

export function validateHostConfig(host: HostScriptConfig): string | null {
  if (!Number.isInteger(host.timeoutSeconds) || host.timeoutSeconds < 1 || host.timeoutSeconds > 60) return "超时必须为 1–60 秒。";
  for (const path of [host.interpreter, host.workingDirectory]) {
    if (path && !/^(\/|[a-zA-Z]:[\\/]|\\\\)/.test(path)) return "解释器和工作目录必须使用绝对路径。";
    if (new TextEncoder().encode(path).length > 4096 || /[\x00-\x1f\x7f-\x9f]/.test(path)) return "路径过长或包含控制字符。";
  }
  if (host.parameters.length > 32) return "最多支持 32 个参数。";
  const names = new Set<string>();
  for (const parameter of host.parameters) {
    if (!/^[A-Za-z_][A-Za-z0-9_]{0,63}$/.test(parameter.name) || names.has(parameter.name.toLowerCase())) return "参数名必须唯一（不区分大小写），只能包含字母、数字和下划线，且不能以数字开头。";
    names.add(parameter.name.toLowerCase());
    if (new TextEncoder().encode(parameter.description).length > 1024 || /[\x00-\x1f\x7f-\x9f]/.test(parameter.description)) return "参数说明过长或包含控制字符。";
  }
  return null;
}
