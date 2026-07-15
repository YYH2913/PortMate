export type TerminalCompletionPreviewMode = "none" | "input" | "top";

export type TerminalCompletionPreferences = {
  enabled: boolean;
  commandNames: boolean;
  commandOptions: boolean;
  commandArgs: boolean;
  history: boolean;
  quickCommands: boolean;
  triggerCharacters: 1 | 2 | 3;
  listRows: 5 | 7 | 10;
  previewMode: TerminalCompletionPreviewMode;
};

export type TerminalCompletionQuickCommand = {
  id: string;
  label: string;
  command: string;
};

export const defaultTerminalCompletionPreferences: TerminalCompletionPreferences = {
  enabled: true,
  commandNames: true,
  commandOptions: true,
  commandArgs: true,
  history: true,
  quickCommands: true,
  triggerCharacters: 1,
  listRows: 7,
  previewMode: "none",
};

export function normalizeTerminalCompletionPreferences(value: unknown): TerminalCompletionPreferences {
  const source = value && typeof value === "object" && !Array.isArray(value)
    ? value as Partial<Record<keyof TerminalCompletionPreferences, unknown>>
    : {};
  return {
    enabled: booleanValue(source.enabled, true),
    commandNames: booleanValue(source.commandNames, true),
    commandOptions: booleanValue(source.commandOptions, true),
    commandArgs: booleanValue(source.commandArgs, true),
    history: booleanValue(source.history, true),
    quickCommands: booleanValue(source.quickCommands, true),
    triggerCharacters: source.triggerCharacters === 2 || source.triggerCharacters === 3
      ? source.triggerCharacters
      : 1,
    listRows: source.listRows === 5 || source.listRows === 10 ? source.listRows : 7,
    previewMode: source.previewMode === "input" || source.previewMode === "top"
      ? source.previewMode
      : "none",
  };
}

export function terminalCompletionPreferencesFromSettings(value: unknown): TerminalCompletionPreferences {
  const source = value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {};
  return normalizeTerminalCompletionPreferences({
    enabled: source.completionEnabled,
    commandNames: source.completionCommandNames,
    commandOptions: source.completionCommandOptions,
    commandArgs: source.completionCommandArgs,
    history: source.completionHistory,
    quickCommands: source.completionQuickCommands,
    triggerCharacters: numericSetting(source.completionTriggerChars, [1, 2, 3]),
    listRows: numericSetting(source.completionListHeight, [5, 7, 10]),
    previewMode: source.completionPreviewMode === "输入框"
      ? "input"
      : source.completionPreviewMode === "列表顶部"
        ? "top"
        : "none",
  });
}

function booleanValue(value: unknown, fallback: boolean): boolean {
  return typeof value === "boolean" ? value : fallback;
}

function numericSetting(value: unknown, allowed: readonly number[]): number | undefined {
  if (typeof value !== "string" && typeof value !== "number") return undefined;
  const parsed = Number.parseInt(String(value), 10);
  return allowed.includes(parsed) ? parsed : undefined;
}
