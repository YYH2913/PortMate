import type { TerminalCompletionPreferences, TerminalCompletionQuickCommand } from "./terminal-completion-prefs";
import {
  terminalCommandCatalog,
  terminalCommandSchema,
  terminalCommandSubcommand,
} from "./terminal-command-catalog";
import type {
  TerminalCommandCatalogEntry,
  TerminalCommandSchema,
} from "./terminal-command-catalog";
export type { TerminalCompletionPreferences, TerminalCompletionQuickCommand } from "./terminal-completion-prefs";

export type TerminalCompletionSource = "command" | "subcommand" | "option" | "argument" | "history" | "quick";

export type TerminalCompletionSuggestion = {
  id: string;
  source: TerminalCompletionSource;
  label: string;
  detail: string;
  target: string;
  appendText: string;
};

export type TerminalCompletionInputState = {
  line: string;
  synchronized: boolean;
};

export type TerminalCompletionUsageHint = {
  label: string;
  detail: string;
};

export const emptyTerminalCompletionInputState: TerminalCompletionInputState = {
  line: "",
  synchronized: true,
};

const MAX_COMPLETION_LINE_CHARACTERS = 512;
const MAX_COMPLETION_CANDIDATES = 40;
const supportedTerminalKinds = new Set(["serial", "shell", "ssh", "tcp", "telnet", "tmux"]);

export function reduceTerminalCompletionInput(
  current: TerminalCompletionInputState,
  text: string,
): TerminalCompletionInputState {
  let line = current.line;
  let synchronized = current.synchronized;
  for (const character of text) {
    if (character === "\r" || character === "\n" || character === "\u0003" || character === "\u0004") {
      line = "";
      synchronized = true;
      continue;
    }
    if (!synchronized) continue;
    if (character === "\b" || character === "\u007f") {
      line = Array.from(line).slice(0, -1).join("");
      continue;
    }
    if (character === "\u0015") {
      line = "";
      continue;
    }
    if (character === "\u0017") {
      line = line.replace(/\S+\s*$/, "");
      continue;
    }
    if (character >= " " && character !== "\u007f") {
      if (Array.from(line).length >= MAX_COMPLETION_LINE_CHARACTERS) {
        line = "";
        synchronized = false;
      } else {
        line += character;
      }
      continue;
    }
    line = "";
    synchronized = false;
  }
  return { line, synchronized };
}

export function terminalCompletionSuggestions({
  line,
  preferences,
  history = [],
  quickCommands = [],
}: {
  line: string;
  preferences: TerminalCompletionPreferences;
  history?: readonly string[];
  quickCommands?: readonly TerminalCompletionQuickCommand[];
}): TerminalCompletionSuggestion[] {
  if (!preferences.enabled || !completionLineIsSafe(line)) return [];
  const tokenMatch = line.match(/\S*$/);
  const token = tokenMatch?.[0] ?? "";
  const beforeToken = line.slice(0, line.length - token.length);
  const tokens = line.trimStart().split(/\s+/).filter(Boolean);
  const command = tokens[0] ?? "";
  const typedCharacters = token.length || command.length;
  if (typedCharacters < preferences.triggerCharacters) return [];

  const candidates: TerminalCompletionSuggestion[] = [];
  if (preferences.quickCommands) {
    for (const quick of quickCommands) {
      const target = normalizeCandidateLine(quick.command);
      if (!target || !target.startsWith(line) || target === line) continue;
      candidates.push(suggestion(`quick:${quick.id}`, "quick", quick.label, "快速命令", target, line, false));
    }
  }
  if (preferences.history) {
    for (let index = 0; index < history.length; index += 1) {
      const target = normalizeCandidateLine(history[index]);
      if (!target || !target.startsWith(line) || target === line) continue;
      candidates.push(suggestion(`history:${index}:${target}`, "history", target, "历史命令", target, line, false));
    }
  }

  if (tokens.length <= 1 && !beforeToken.trim() && preferences.commandNames) {
    const indentation = line.slice(0, line.length - line.trimStart().length);
    for (const entry of terminalCommandCatalog) {
      const target = `${indentation}${entry.value}`;
      if (!target.startsWith(line)) continue;
      candidates.push(suggestion(`command:${entry.value}`, "command", entry.value, entry.detail, target, line, true));
    }
  }

  if (command && beforeToken.trim()) {
    const root = terminalCommandSchema(command);
    const context = root ? resolveCommandContext(root, completedTokens(beforeToken).slice(1)) : null;
    if (preferences.commandOptions && (!token || token.startsWith("-"))) {
      for (const entry of context ? commandOptions(root!, context) : []) {
        pushCatalogSuggestion(candidates, entry, "option", command, beforeToken, line);
      }
    }
    if (preferences.commandArgs && !token.startsWith("-") && context) {
      for (const entry of context.subcommands) {
        pushCatalogSuggestion(candidates, entry, "subcommand", command, beforeToken, line);
      }
    }
    if (preferences.commandArgs && !token.startsWith("-") && context) {
      for (const entry of context.arguments) {
        pushCatalogSuggestion(candidates, entry, "argument", command, beforeToken, line);
      }
    }
  }

  const seenTargets = new Set<string>();
  return candidates
    .sort((left, right) => sourcePriority(left.source) - sourcePriority(right.source)
      || left.target.length - right.target.length
      || left.label.localeCompare(right.label))
    .filter((candidate) => {
      if (!candidate.appendText || seenTargets.has(candidate.target)) return false;
      seenTargets.add(candidate.target);
      return true;
    })
    .slice(0, MAX_COMPLETION_CANDIDATES);
}

export function terminalCompletionUsageHint({
  line,
  preferences,
}: {
  line: string;
  preferences: TerminalCompletionPreferences;
}): TerminalCompletionUsageHint | null {
  if (!preferences.enabled
    || (!preferences.commandArgs && !preferences.commandOptions)
    || !completionLineIsSafe(line)) return null;
  const tokens = line.trimStart().split(/\s+/).filter(Boolean);
  const root = terminalCommandSchema(tokens[0] ?? "");
  if (!root) return null;
  const context = resolveCommandContext(root, tokens.slice(1));
  return { label: context.usage, detail: context.detail };
}

export function terminalCompletionSupported(value: unknown): boolean {
  return typeof value === "string" && supportedTerminalKinds.has(value);
}

export function terminalCompletionSourceLabel(source: TerminalCompletionSource): string {
  if (source === "command") return "命令";
  if (source === "subcommand") return "子命令";
  if (source === "option") return "选项";
  if (source === "argument") return "参数";
  if (source === "history") return "历史";
  return "Quick";
}

function completionLineIsSafe(line: string): boolean {
  return Boolean(line.trim())
    && line.length <= MAX_COMPLETION_LINE_CHARACTERS
    && !/[\u0000-\u001f\u007f'"`\\$(){}[\];|&<>]/.test(line);
}

function normalizeCandidateLine(value: unknown): string {
  if (typeof value !== "string") return "";
  const target = value.trim();
  return completionLineIsSafe(target) ? target : "";
}

function suggestion(
  id: string,
  source: TerminalCompletionSource,
  label: string,
  detail: string,
  target: string,
  line: string,
  completeWithSpace: boolean,
): TerminalCompletionSuggestion {
  const trailingSpace = completeWithSpace && !target.endsWith(" ") ? " " : "";
  const appendText = `${target.slice(line.length)}${trailingSpace}`;
  return { id, source, label, detail, target, appendText };
}

function sourcePriority(source: TerminalCompletionSource): number {
  if (source === "quick") return 0;
  if (source === "history") return 1;
  if (source === "subcommand") return 2;
  if (source === "argument") return 3;
  if (source === "option") return 4;
  return 5;
}

function completedTokens(beforeToken: string): string[] {
  return beforeToken.trim().split(/\s+/).filter(Boolean);
}

function resolveCommandContext(
  root: TerminalCommandSchema,
  tokens: readonly string[],
): TerminalCommandSchema {
  let context = root;
  for (const token of tokens) {
    if (!token || token.startsWith("-")) continue;
    const subcommand = terminalCommandSubcommand(context, token);
    if (subcommand) context = subcommand;
  }
  return context;
}

function commandOptions(
  root: TerminalCommandSchema,
  context: TerminalCommandSchema,
): TerminalCommandCatalogEntry[] {
  const seen = new Set<string>();
  return [...context.options, ...(context === root ? [] : root.options)].filter((entry) => {
    if (seen.has(entry.value)) return false;
    seen.add(entry.value);
    return true;
  });
}

function pushCatalogSuggestion(
  candidates: TerminalCompletionSuggestion[],
  entry: TerminalCommandCatalogEntry,
  source: "subcommand" | "option" | "argument",
  command: string,
  beforeToken: string,
  line: string,
) {
  const target = `${beforeToken}${entry.value}`;
  if (!target.startsWith(line)) return;
  candidates.push(suggestion(
    `${source}:${command}:${entry.value}`,
    source,
    entry.value,
    entry.detail,
    target,
    line,
    true,
  ));
}
