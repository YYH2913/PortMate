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

export type TerminalCompletionInputReduction = {
  state: TerminalCompletionInputState;
  submittedCommands: string[];
};

export type TerminalCompletionUsageHint = {
  label: string;
  detail: string;
};

export type TerminalCompletionHistoryEntry = {
  index: number;
  target: string;
};

export type TerminalCompletionHistoryIndex = {
  entries: readonly TerminalCompletionHistoryEntry[];
  prefixes: ReadonlyMap<string, readonly TerminalCompletionHistoryEntry[]>;
};

type TerminalCommandContext = {
  schema: TerminalCommandSchema;
  unknownSubcommandPath: readonly string[] | null;
};

type ResolveCommandContextOptions = {
  trailingTokenComplete?: boolean;
};

export const emptyTerminalCompletionInputState: TerminalCompletionInputState = {
  line: "",
  synchronized: true,
};

const HISTORY_INDEX_PREFIX_CHARACTERS = 2;

/** Normalize and prefix-index history once per snapshot for interactive completion. */
export function indexTerminalCompletionHistory(
  history: readonly string[],
): TerminalCompletionHistoryIndex {
  const entries: TerminalCompletionHistoryEntry[] = [];
  const prefixes = new Map<string, TerminalCompletionHistoryEntry[]>();
  for (let index = 0; index < history.length; index += 1) {
    const target = normalizeCandidateLine(history[index]);
    if (!target) continue;
    const entry = { index, target };
    entries.push(entry);
    const characters = Array.from(target);
    for (let length = 1; length <= Math.min(HISTORY_INDEX_PREFIX_CHARACTERS, characters.length); length += 1) {
      const prefix = characters.slice(0, length).join("");
      const bucket = prefixes.get(prefix);
      if (bucket) bucket.push(entry);
      else prefixes.set(prefix, [entry]);
    }
  }
  return { entries, prefixes };
}

const MAX_COMPLETION_LINE_CHARACTERS = 512;
const MAX_COMPLETION_CANDIDATES = 40;
const supportedTerminalKinds = new Set(["serial", "shell", "ssh", "tcp", "telnet", "tmux"]);

export function reduceTerminalCompletionInput(
  current: TerminalCompletionInputState,
  text: string,
): TerminalCompletionInputState {
  return reduceTerminalCompletionInputWithSubmissions(current, text).state;
}

export function reduceTerminalCompletionInputWithSubmissions(
  current: TerminalCompletionInputState,
  text: string,
): TerminalCompletionInputReduction {
  let line = current.line;
  let synchronized = current.synchronized;
  const submittedCommands: string[] = [];
  for (const character of text) {
    if (character === "\r" || character === "\n") {
      if (synchronized && line.trim()) submittedCommands.push(line);
      line = "";
      synchronized = true;
      continue;
    }
    if (character === "\u0003" || character === "\u0004") {
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
  return { state: { line, synchronized }, submittedCommands };
}

export function terminalCompletionSuggestions({
  line,
  preferences,
  history = [],
  historyIndex,
  quickCommands = [],
}: {
  line: string;
  preferences: TerminalCompletionPreferences;
  history?: readonly string[];
  historyIndex?: TerminalCompletionHistoryIndex;
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
    const historyEntries = historyIndex
      ? historyIndex.prefixes.get(historyPrefix(line)) ?? []
      : history.map((value, index) => ({ index, target: normalizeCandidateLine(value) }))
        .filter((entry): entry is TerminalCompletionHistoryEntry => Boolean(entry.target));
    for (const entry of historyEntries) {
      const target = entry.target;
      if (!target || !target.startsWith(line) || target === line) continue;
      candidates.push(suggestion("history:" + entry.index + ":" + target, "history", target, "历史命令", target, line, false));
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
    const root = terminalCommandSchemaForToken(command);
    const resolution = root ? resolveCommandContext(root, completedTokens(beforeToken).slice(1)) : null;
    const context = resolution?.unknownSubcommandPath ? null : resolution?.schema ?? null;
    if (preferences.commandOptions && (!token || token.startsWith("-") || token.startsWith("/"))) {
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
  const commandToken = tokens[0] ?? "";
  const root = terminalCommandSchemaForToken(commandToken);
  if (!root) {
    return commandHasArgumentBoundary(line)
      ? { label: `${commandToken} [参数...]`, detail: "当前环境命令" }
      : null;
  }
  const context = resolveCommandContext(root, tokens.slice(1), {
    trailingTokenComplete: /\s$/.test(line),
  });
  if (context.unknownSubcommandPath) {
    return {
      label: `${[commandToken, ...context.unknownSubcommandPath].join(" ")} [参数...]`,
      detail: context.schema.detail,
    };
  }
  return {
    label: usageForCommandToken(context.schema.usage, root.value, commandToken),
    detail: context.schema.detail,
  };
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

function historyPrefix(line: string): string {
  return Array.from(line).slice(0, HISTORY_INDEX_PREFIX_CHARACTERS).join("");
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
  { trailingTokenComplete = true }: ResolveCommandContextOptions = {},
): TerminalCommandContext {
  let context = root;
  let optionValuePending = false;
  const commandPath: string[] = [];
  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index];
    if (!token) continue;
    if (optionValuePending) {
      optionValuePending = false;
      continue;
    }
    const availableOptions = commandOptions(root, context);
    const optionName = terminalOptionName(token, availableOptions);
    if (token.startsWith("-") || optionName) {
      const option = optionName
        ? availableOptions.find((entry) => entry.value === optionName)
        : undefined;
      optionValuePending = Boolean(option?.takesValue && !token.includes("="));
      continue;
    }
    const subcommand = terminalCommandSubcommand(context, token);
    if (subcommand) {
      context = subcommand;
      commandPath.push(subcommand.value);
      continue;
    }
    if (context.subcommands.length) {
      if (!trailingTokenComplete && index === tokens.length - 1) break;
      return {
        schema: context,
        unknownSubcommandPath: [...commandPath, token],
      };
    }
  }
  return { schema: context, unknownSubcommandPath: null };
}

function terminalCommandSchemaForToken(token: string): TerminalCommandSchema | null {
  const basename = token.slice(token.lastIndexOf("/") + 1);
  const executable = basename.toLowerCase().endsWith(".exe")
    ? basename.slice(0, -4).toLowerCase()
    : basename;
  return terminalCommandSchema(executable);
}

function terminalOptionName(
  token: string,
  availableOptions: readonly TerminalCommandCatalogEntry[],
): string | null {
  if (token.startsWith("-")) return token.split("=", 1)[0];
  if (!token.startsWith("/")) return null;
  return availableOptions.find((entry) => entry.value === token)?.value ?? null;
}

function commandHasArgumentBoundary(line: string): boolean {
  return /\s/.test(line.trimStart());
}

function usageForCommandToken(usage: string, catalogCommand: string, commandToken: string): string {
  if (catalogCommand === commandToken) return usage;
  if (usage === catalogCommand) return commandToken;
  return usage.startsWith(`${catalogCommand} `)
    ? `${commandToken}${usage.slice(catalogCommand.length)}`
    : usage;
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
