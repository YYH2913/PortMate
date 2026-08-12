export type TerminalSemanticTokenKind =
  | "command"
  | "option"
  | "string"
  | "path"
  | "address"
  | "number"
  | "variable"
  | "operator";

export type TerminalSemanticToken = {
  kind: TerminalSemanticTokenKind;
  start: number;
  end: number;
};

export const MAX_TERMINAL_SEMANTIC_LINE_CHARACTERS = 1_024;

type ShellWord = {
  start: number;
  end: number;
  simpleValue: string | null;
};

type CommandWrapper = {
  name: string;
  positionalValues: number;
  optionValue: boolean;
  optionsEnded: boolean;
};

type ShellScanMode = "commands" | "arguments";
type ShellStopCharacter = ")" | "`" | null;

const commandSeparators = new Set([";", ";;", ";&", ";;&", "|", "|&", "||", "&&", "&"]);
const redirectionOperators = new Set(["<", ">", "<<", "<<-", "<<<", ">>", "><", "<>", ">&", "<&", ">|", "&>", "&>>"]);
const specialPromptMarkers = new Set(["❯", "➜", "λ", "➤", "»"]);
const supportedTerminalKinds = new Set(["serial", "shell", "ssh", "tcp", "telnet", "tmux"]);

const wrapperPositionals: Readonly<Record<string, number>> = {
  sudo: 0,
  doas: 0,
  pkexec: 0,
  env: 0,
  command: 0,
  builtin: 0,
  exec: 0,
  nohup: 0,
  time: 0,
  nice: 0,
  ionice: 0,
  setsid: 0,
  stdbuf: 0,
  timeout: 1,
  chroot: 1,
  busybox: 0,
  xargs: 0,
  watch: 0,
  if: 0,
  then: 0,
  elif: 0,
  while: 0,
  until: 0,
  do: 0,
  else: 0,
  not: 0,
  "!": 0,
};

const wrapperValueOptions: Readonly<Record<string, ReadonlySet<string>>> = {
  sudo: new Set(["-u", "--user", "-g", "--group", "-h", "--host", "-p", "--prompt", "-C", "--close-from", "-R", "--chroot", "-T", "--command-timeout"]),
  doas: new Set(["-u"]),
  pkexec: new Set(["--user"]),
  env: new Set(["-u", "--unset", "-C", "--chdir", "-S", "--split-string"]),
  nice: new Set(["-n", "--adjustment"]),
  ionice: new Set(["-c", "--class", "-n", "--classdata", "-t", "--ignore"]),
  stdbuf: new Set(["-i", "--input", "-o", "--output", "-e", "--error"]),
  timeout: new Set(["-k", "--kill-after", "-s", "--signal"]),
  chroot: new Set(["--userspec", "--groups"]),
  xargs: new Set(["-a", "--arg-file", "-d", "--delimiter", "-E", "--eof", "-I", "--replace", "-L", "--max-lines", "-n", "--max-args", "-P", "--max-procs", "-s", "--max-chars"]),
  watch: new Set(["-n", "--interval", "-x", "--exec"]),
};

export function terminalSemanticHighlightingEnabled(value: unknown): boolean {
  if (!value || typeof value !== "object" || Array.isArray(value)) return true;
  const enabled = (value as { semanticHighlightingEnabled?: unknown }).semanticHighlightingEnabled;
  return typeof enabled === "boolean" ? enabled : true;
}

export function terminalSemanticHighlightingSupported(value: unknown): boolean {
  return typeof value === "string" && supportedTerminalKinds.has(value);
}

export function terminalSemanticTokens(line: string): TerminalSemanticToken[] {
  const characters = Array.from(line);
  if (!characters.length || characters.length > MAX_TERMINAL_SEMANTIC_LINE_CHARACTERS) return [];
  const commandStart = terminalPromptCommandStart(characters);
  if (commandStart === null || commandStart >= characters.length) return [];

  const tokens: TerminalSemanticToken[] = [];
  scanShell(characters, commandStart, tokens, "commands", null);
  return normalizeTokens(tokens, characters.length);
}

function scanShell(
  characters: readonly string[],
  start: number,
  tokens: TerminalSemanticToken[],
  mode: ShellScanMode,
  stopCharacter: ShellStopCharacter,
): number {
  let index = start;
  let expectingCommand = mode === "commands";
  let wrapper: CommandWrapper | null = null;
  let redirectionTarget = false;
  let adjacentAssignmentEnd = -1;

  while (index < characters.length) {
    const character = characters[index];
    if (stopCharacter && character === stopCharacter) {
      tokens.push({ kind: stopCharacter === ")" ? "operator" : "string", start: index, end: index + 1 });
      return index + 1;
    }
    if (/\s/u.test(character)) {
      adjacentAssignmentEnd = -1;
      index += 1;
      continue;
    }

    const processSubstitution = (character === "<" || character === ">") && characters[index + 1] === "(";
    if (processSubstitution) {
      tokens.push({ kind: "operator", start: index, end: index + 2 });
      index = scanShell(characters, index + 2, tokens, "commands", ")");
      redirectionTarget = false;
      adjacentAssignmentEnd = -1;
      continue;
    }

    const operator = shellOperatorAt(characters, index);
    if (operator) {
      const end = index + Array.from(operator).length;
      if (operator === "(" && adjacentAssignmentEnd === index) {
        tokens.push({ kind: "operator", start: index, end });
        index = scanShell(characters, end, tokens, "arguments", ")");
        expectingCommand = false;
        wrapper = null;
        adjacentAssignmentEnd = -1;
        continue;
      }
      if (operator === "(") {
        tokens.push({ kind: "operator", start: index, end });
        index = scanShell(characters, end, tokens, mode, ")");
        expectingCommand = false;
        wrapper = null;
        adjacentAssignmentEnd = -1;
        continue;
      }

      tokens.push({ kind: "operator", start: index, end });
      if (commandSeparators.has(operator)) {
        expectingCommand = mode === "commands";
        wrapper = null;
        redirectionTarget = false;
      } else if (isRedirectionOperator(operator)) {
        redirectionTarget = true;
      } else if (operator === "{") {
        expectingCommand = mode === "commands";
        wrapper = null;
      } else if (operator === "}") {
        expectingCommand = false;
        wrapper = null;
      }
      adjacentAssignmentEnd = -1;
      index = end;
      continue;
    }

    const word = readShellWord(characters, index, tokens);
    if (word.end <= index) {
      index += 1;
      continue;
    }
    const assignmentEnd = environmentAssignmentEnd(characters, word);
    if (mode === "arguments") {
      addArgumentToken(tokens, word);
    } else if (redirectionTarget) {
      addArgumentToken(tokens, word);
      redirectionTarget = false;
    } else if (assignmentEnd !== null) {
      addAssignmentToken(tokens, word, assignmentEnd);
    } else if (expectingCommand) {
      const consumed = consumeExpectedCommand(tokens, word, wrapper);
      wrapper = consumed.wrapper;
      expectingCommand = consumed.expectingCommand;
    } else {
      addArgumentToken(tokens, word);
    }
    adjacentAssignmentEnd = assignmentEnd === null ? -1 : word.end;
    index = word.end;
  }
  return index;
}

function consumeExpectedCommand(
  tokens: TerminalSemanticToken[],
  word: ShellWord,
  wrapper: CommandWrapper | null,
): { wrapper: CommandWrapper | null; expectingCommand: boolean } {
  if (wrapper?.optionValue) {
    addArgumentToken(tokens, word);
    return { wrapper: { ...wrapper, optionValue: false }, expectingCommand: true };
  }

  const value = word.simpleValue;
  if (wrapper && !wrapper.optionsEnded && value?.startsWith("-")) {
    tokens.push({ kind: "option", start: word.start, end: word.end });
    if (value === "--") return { wrapper: { ...wrapper, optionsEnded: true }, expectingCommand: true };
    return {
      wrapper: { ...wrapper, optionValue: wrapperOptionRequiresValue(wrapper.name, value) },
      expectingCommand: true,
    };
  }
  if (wrapper && wrapper.positionalValues > 0) {
    addArgumentToken(tokens, word);
    return {
      wrapper: { ...wrapper, positionalValues: wrapper.positionalValues - 1 },
      expectingCommand: true,
    };
  }

  if (value !== null) tokens.push({ kind: "command", start: word.start, end: word.end });
  const nextWrapper = value === null ? null : commandWrapper(value);
  return { wrapper: nextWrapper, expectingCommand: nextWrapper !== null };
}

function commandWrapper(value: string): CommandWrapper | null {
  const positionalValues = wrapperPositionals[value];
  return positionalValues === undefined
    ? null
    : { name: value, positionalValues, optionValue: false, optionsEnded: false };
}

function wrapperOptionRequiresValue(wrapper: string, value: string): boolean {
  if (value.includes("=")) return false;
  if (wrapperValueOptions[wrapper]?.has(value)) return true;
  return false;
}

function readShellWord(
  characters: readonly string[],
  start: number,
  tokens: TerminalSemanticToken[],
): ShellWord {
  let index = start;
  let simpleValue = "";
  let simple = true;

  while (index < characters.length) {
    const character = characters[index];
    if (/\s/u.test(character) || shellOperatorAt(characters, index)) break;
    if (character === "\\") {
      if (index + 1 < characters.length) {
        simpleValue += character + characters[index + 1];
        index += 2;
      } else {
        simpleValue += character;
        index += 1;
      }
      continue;
    }
    if (character === "'") {
      simple = false;
      const end = singleQuotedTokenEnd(characters, index);
      tokens.push({ kind: "string", start: index, end });
      index = end;
      continue;
    }
    if (character === '"') {
      simple = false;
      index = readDoubleQuotedWord(characters, index, tokens);
      continue;
    }
    if (character === "`") {
      simple = false;
      tokens.push({ kind: "string", start: index, end: index + 1 });
      index = scanShell(characters, index + 1, tokens, "commands", "`");
      continue;
    }
    if (character === "$" && characters[index + 1] === "(") {
      simple = false;
      tokens.push({ kind: "operator", start: index, end: index + 2 });
      index = scanShell(characters, index + 2, tokens, "commands", ")");
      continue;
    }
    if (character === "$" && index + 1 < characters.length) {
      const end = variableTokenEnd(characters, index);
      if (end > index + 1) {
        simple = false;
        tokens.push({ kind: "variable", start: index, end });
        index = end;
        continue;
      }
    }
    if (character === "%") {
      const end = cmdVariableTokenEnd(characters, index, "%");
      if (end > index + 1) {
        simple = false;
        tokens.push({ kind: "variable", start: index, end });
        index = end;
        continue;
      }
    }
    if (character === "!") {
      const end = cmdVariableTokenEnd(characters, index, "!");
      if (end > index + 1) {
        simple = false;
        tokens.push({ kind: "variable", start: index, end });
        index = end;
        continue;
      }
    }
    simpleValue += character;
    index += 1;
  }
  return { start, end: Math.max(start + 1, index), simpleValue: simple ? simpleValue : null };
}

function readDoubleQuotedWord(
  characters: readonly string[],
  start: number,
  tokens: TerminalSemanticToken[],
): number {
  let index = start + 1;
  let stringStart = start;
  while (index < characters.length) {
    const character = characters[index];
    if (character === "\\" && index + 1 < characters.length) {
      index += 2;
      continue;
    }
    if (character === '"') {
      addStringToken(tokens, stringStart, index + 1);
      return index + 1;
    }
    if (character === "$" && characters[index + 1] === "(") {
      addStringToken(tokens, stringStart, index);
      tokens.push({ kind: "operator", start: index, end: index + 2 });
      index = scanShell(characters, index + 2, tokens, "commands", ")");
      stringStart = index;
      continue;
    }
    if (character === "$" && index + 1 < characters.length) {
      const end = variableTokenEnd(characters, index);
      if (end > index + 1) {
        addStringToken(tokens, stringStart, index);
        tokens.push({ kind: "variable", start: index, end });
        index = end;
        stringStart = index;
        continue;
      }
    }
    if (character === "`") {
      addStringToken(tokens, stringStart, index);
      tokens.push({ kind: "string", start: index, end: index + 1 });
      index = scanShell(characters, index + 1, tokens, "commands", "`");
      stringStart = index;
      continue;
    }
    index += 1;
  }
  addStringToken(tokens, stringStart, characters.length);
  return characters.length;
}

function addStringToken(tokens: TerminalSemanticToken[], start: number, end: number) {
  if (end > start) tokens.push({ kind: "string", start, end });
}

function singleQuotedTokenEnd(characters: readonly string[], start: number): number {
  for (let index = start + 1; index < characters.length; index += 1) {
    if (characters[index] === "'") return index + 1;
  }
  return characters.length;
}

function variableTokenEnd(characters: readonly string[], start: number): number {
  if (characters[start + 1] === "{") {
    let depth = 1;
    for (let index = start + 2; index < characters.length; index += 1) {
      if (characters[index] === "{") depth += 1;
      if (characters[index] !== "}") continue;
      depth -= 1;
      if (depth === 0) return index + 1;
    }
    return characters.length;
  }
  let index = start + 1;
  if (/[?@#$!^*_-]/u.test(characters[index] ?? "")) return index + 1;
  while (index < characters.length && /[\p{L}\p{N}_]/u.test(characters[index])) index += 1;
  if (characters[index] === ":" && index > start + 1) {
    index += 1;
    while (index < characters.length && /[\p{L}\p{N}_-]/u.test(characters[index])) index += 1;
  }
  return index;
}

function cmdVariableTokenEnd(characters: readonly string[], start: number, delimiter: "%" | "!"): number {
  for (let index = start + 1; index < characters.length; index += 1) {
    if (characters[index] === delimiter) return index > start + 1 ? index + 1 : start + 1;
    if (/\s/u.test(characters[index])) break;
  }
  return start + 1;
}

function shellOperatorAt(characters: readonly string[], start: number): string | null {
  const remainder = characters.slice(start, start + 5).join("");
  const descriptorRedirection = remainder.match(/^\d+(?:>>|<<|<>|>&|<&|>|<)/u)?.[0];
  if (descriptorRedirection) return descriptorRedirection;
  for (const operator of ["&>>", ";;&", "<<<", "<<-", "&&", "||", ">>", "<<", "><", "<>", ">&", "<&", ">|", "&>", "|&", ";;", ";&"]) {
    if (remainder.startsWith(operator)) return operator;
  }
  return ";|&<>(){}".includes(characters[start] ?? "") ? characters[start] : null;
}

function isRedirectionOperator(value: string): boolean {
  return redirectionOperators.has(value) || /^\d+(?:>>|<<|<>|>&|<&|>|<)$/u.test(value);
}

function environmentAssignmentEnd(characters: readonly string[], word: ShellWord): number | null {
  const raw = characters.slice(word.start, word.end).join("");
  const match = raw.match(/^[A-Za-z_][A-Za-z0-9_]*(?:\+)?=/u)?.[0];
  return match ? word.start + Array.from(match).length : null;
}

function addAssignmentToken(tokens: TerminalSemanticToken[], word: ShellWord, assignmentEnd: number) {
  tokens.push({
    kind: "variable",
    start: word.start,
    end: word.simpleValue === null ? assignmentEnd : word.end,
  });
}

function addArgumentToken(tokens: TerminalSemanticToken[], word: ShellWord) {
  if (word.simpleValue === null) return;
  const kind = classifyBareArgument(word.simpleValue);
  if (kind) tokens.push({ kind, start: word.start, end: word.end });
}

function classifyBareArgument(value: string): TerminalSemanticTokenKind | null {
  if (/^(?:=|==|!=|\+|-|\*|\?|:)$/u.test(value)) return "operator";
  if (/^--?[\p{L}\p{N}][\p{L}\p{N}_-]*(?:=.*)?$/u.test(value) || /^\/[A-Za-z?]$/u.test(value)) return "option";
  if (networkAddress(value)) return "address";
  if (/^[+-]?(?:0x[\dA-Fa-f]+|\d+(?:\.\d+)?(?:e[+-]?\d+)?)(?:%|ns|us|ms|s|m|h|d|B|KiB|MiB|GiB)?$/iu.test(value)) return "number";
  if (pathArgument(value)) return "path";
  return null;
}

function networkAddress(value: string): boolean {
  if (/^[A-Za-z][A-Za-z0-9+.-]*:\/\//u.test(value)) return true;
  if (/^[\w.-]+@[\w.-]+:(?:[/~].*)?$/u.test(value)) return true;
  if (/^(?:[\dA-Fa-f]{2}:){5}[\dA-Fa-f]{2}$/u.test(value)) return true;
  const bracketedIpv6 = value.match(/^\[([\dA-Fa-f:]+)\](?::(\d{1,5}))?$/u);
  if (bracketedIpv6) return ipv6Address(bracketedIpv6[1]) && validPort(bracketedIpv6[2]);
  const ipv4Cidr = value.match(/^(.+?)(?:\/(\d{1,2}))?(?::(\d{1,5}))?$/u);
  if (ipv4Cidr && ipv4Address(ipv4Cidr[1])) {
    return (!ipv4Cidr[2] || Number(ipv4Cidr[2]) <= 32) && validPort(ipv4Cidr[3]);
  }
  const ipv6Cidr = value.match(/^([\dA-Fa-f:]+)(?:\/(\d{1,3}))?$/u);
  if (ipv6Cidr && ipv6Address(ipv6Cidr[1])) return !ipv6Cidr[2] || Number(ipv6Cidr[2]) <= 128;
  const hostPort = value.match(/^([\p{L}\p{N}](?:[\p{L}\p{N}.-]*[\p{L}\p{N}])?):(\d{1,5})$/u);
  return Boolean(hostPort && validPort(hostPort[2]));
}

function validPort(value: string | undefined): boolean {
  return value === undefined || Number(value) <= 65_535;
}

function pathArgument(value: string): boolean {
  if (value === "/" || value === "~" || value === "." || value === "..") return true;
  if (/^(?:~|\.{1,2})?[/\\][^\s]+$/u.test(value)) return true;
  if (/^[A-Za-z]:[\\/][^\s]*$/u.test(value) || /^(?:\\\\|\/\/)[^\s]+$/u.test(value)) return true;
  if (/^(?:[^\s/\\]+[/\\])+[^\s/\\]+$/u.test(value)) return true;
  return /^[^\s/\\]+\.[A-Za-z0-9_-]{1,12}$/u.test(value);
}

function ipv4Address(value: string): boolean {
  const parts = value.split(".");
  return parts.length === 4 && parts.every((part) => /^\d{1,3}$/u.test(part) && Number(part) <= 255);
}

function ipv6Address(value: string): boolean {
  return value.includes(":") && /^[\dA-Fa-f:]+$/u.test(value) && value.split(":").length >= 3;
}

function terminalPromptCommandStart(characters: readonly string[]): number | null {
  for (let index = characters.length - 1; index >= 0; index -= 1) {
    const marker = promptMarkerAt(characters, index);
    if (!marker) continue;
    const markerEnd = index + Array.from(marker).length;
    let commandStart = markerEnd;
    const separated = /\s/u.test(characters[commandStart] ?? "");
    while (commandStart < characters.length && /\s/u.test(characters[commandStart])) commandStart += 1;
    if (commandStart >= characters.length) continue;
    const rawPrefix = characters.slice(0, index).join("");
    if (!validPromptPrefix(rawPrefix, marker, separated)) continue;
    return commandStart;
  }
  return null;
}

function promptMarkerAt(characters: readonly string[], index: number): string | null {
  if (characters[index] === "=" && characters[index + 1] === ">") return "=>";
  const marker = characters[index];
  return "$#>%".includes(marker) || specialPromptMarkers.has(marker) ? marker : null;
}

function validPromptPrefix(rawPrefix: string, marker: string, separated: boolean): boolean {
  if (rawPrefix.length > 256 || /[\u0000-\u001f\u007f-\u009f]/u.test(rawPrefix)) return false;
  const prefix = rawPrefix.trim();
  if (marker === "=>") return prefix.length === 0;
  if (specialPromptMarkers.has(marker)) return true;
  if (!prefix) return separated;
  if (marker === "%" && /\d$/u.test(prefix)) return false;
  if (/\s$/u.test(rawPrefix)) return false;

  const windows = /^(?:PS\s+)?(?:[A-Za-z]:[\\/][^<>|?*\r\n]*|[A-Za-z]:[\\/])$/u.test(prefix);
  const bracketed = /^(?:\([^()\r\n]{1,64}\)\s*)?\[[^\]\r\n]{1,160}\]$/u.test(prefix);
  const device = /^[\p{L}\p{N}_.@~/:\\-]{1,160}(?:\([\p{L}\p{N}_.@~/:\\-]{1,80}\))?$/u.test(prefix);
  const virtualEnvironment = /^\([^()\r\n]{1,64}\)\s+[\p{L}\p{N}_.@~/:\\-]{1,160}$/u.test(prefix);
  const fish = /^[\p{L}\p{N}_.-]+@[\p{L}\p{N}_.-]+\s+(?:~|[/\\][^\s]+)$/u.test(prefix);
  return windows || bracketed || device || virtualEnvironment || fish;
}

function normalizeTokens(tokens: TerminalSemanticToken[], lineLength: number): TerminalSemanticToken[] {
  const sorted = tokens
    .filter((token) => token.start >= 0 && token.end > token.start && token.end <= lineLength)
    .sort((left, right) => left.start - right.start || left.end - right.end);
  const normalized: TerminalSemanticToken[] = [];
  for (const token of sorted) {
    const previous = normalized.at(-1);
    if (previous && previous.kind === token.kind && token.start < previous.end) {
      previous.end = Math.max(previous.end, token.end);
    } else {
      normalized.push({ ...token });
    }
  }
  return normalized;
}
