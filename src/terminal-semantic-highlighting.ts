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

const operatorCharacters = new Set([";", "|", "&", "<", ">", "(", ")", "[", "]", "{", "}"]);
const commandSeparators = new Set([";", "|", "||", "&&"]);
const supportedTerminalKinds = new Set(["serial", "shell", "ssh", "tcp", "telnet", "tmux"]);

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
  let expectingCommand = true;
  let commandWrapper: "sudo" | "env" | null = null;
  let wrapperOptionValue = false;
  let index = commandStart;
  while (index < characters.length) {
    const character = characters[index];
    if (/\s/u.test(character)) {
      index += 1;
      continue;
    }

    if (character === "'" || character === '"' || character === "`") {
      const end = quotedTokenEnd(characters, index, character);
      tokens.push({ kind: "string", start: index, end });
      index = end;
      if (commandWrapper && wrapperOptionValue) wrapperOptionValue = false;
      else expectingCommand = false;
      continue;
    }

    if (character === "$" && index + 1 < characters.length) {
      const end = variableTokenEnd(characters, index);
      tokens.push({ kind: "variable", start: index, end });
      index = end;
      if (commandWrapper && wrapperOptionValue) wrapperOptionValue = false;
      continue;
    }

    if (operatorCharacters.has(character)) {
      const end = operatorTokenEnd(characters, index);
      const operator = characters.slice(index, end).join("");
      tokens.push({ kind: "operator", start: index, end });
      expectingCommand = commandSeparators.has(operator);
      commandWrapper = null;
      wrapperOptionValue = false;
      index = end;
      continue;
    }

    const end = bareTokenEnd(characters, index);
    const value = characters.slice(index, end).join("");
    if (expectingCommand) {
      if (environmentAssignment(value)) {
        tokens.push({ kind: "variable", start: index, end });
      } else if (commandWrapper && wrapperOptionValue) {
        const kind = classifyBareArgument(value);
        if (kind) tokens.push({ kind, start: index, end });
        wrapperOptionValue = false;
      } else if (commandWrapper && value.startsWith("-")) {
        tokens.push({ kind: "option", start: index, end });
        wrapperOptionValue = wrapperOptionRequiresValue(commandWrapper, value);
      } else {
        tokens.push({ kind: "command", start: index, end });
        commandWrapper = value === "sudo" || value === "env" ? value : null;
        expectingCommand = commandWrapper !== null;
      }
    } else {
      const kind = classifyBareArgument(value);
      if (kind) tokens.push({ kind, start: index, end });
    }
    index = end;
  }
  return tokens;
}

function wrapperOptionRequiresValue(wrapper: "sudo" | "env", value: string): boolean {
  if (value.includes("=") || /^-[^-].+/u.test(value)) return false;
  if (wrapper === "env") return ["-u", "--unset", "-C", "--chdir", "-S", "--split-string"].includes(value);
  return [
    "-u", "--user", "-g", "--group", "-h", "--host", "-p", "--prompt",
    "-C", "--close-from", "-R", "--chroot", "-T", "--command-timeout",
  ].includes(value);
}

function terminalPromptCommandStart(characters: string[]): number | null {
  for (let index = characters.length - 2; index >= 0; index -= 1) {
    const marker = characters[index];
    if (!"$#>%❯".includes(marker) || !/\s/u.test(characters[index + 1])) continue;
    const prefix = characters.slice(0, index).join("").trim();
    if (marker === "%" && /\d$/u.test(prefix)) continue;
    if (!validPromptPrefix(prefix, marker)) continue;
    let commandStart = index + 1;
    while (commandStart < characters.length && /\s/u.test(characters[commandStart])) commandStart += 1;
    return commandStart;
  }
  return null;
}

function validPromptPrefix(prefix: string, marker: string): boolean {
  if (prefix.length > 256 || /[\u0000-\u001f\u007f-\u009f]/u.test(prefix)) return false;
  if (!prefix) return true;
  if (marker === ">") return /^(?:PS\s+)?[\w.()@~/:\\\-[\] ]+$/u.test(prefix);
  return /[@~/:\\\-[\]()]/u.test(prefix) || /^[\w.-]{1,64}$/u.test(prefix);
}

function quotedTokenEnd(characters: string[], start: number, quote: string): number {
  let escaped = false;
  for (let index = start + 1; index < characters.length; index += 1) {
    const character = characters[index];
    if (!escaped && character === quote) return index + 1;
    escaped = !escaped && character === "\\";
    if (character !== "\\") escaped = false;
  }
  return characters.length;
}

function variableTokenEnd(characters: string[], start: number): number {
  if (characters[start + 1] === "{") {
    const closing = characters.indexOf("}", start + 2);
    return closing < 0 ? characters.length : closing + 1;
  }
  let index = start + 1;
  while (index < characters.length && /[\p{L}\p{N}_?@#$!*\-]/u.test(characters[index])) index += 1;
  return Math.max(start + 1, index);
}

function operatorTokenEnd(characters: string[], start: number): number {
  const pair = `${characters[start]}${characters[start + 1] ?? ""}`;
  return ["&&", "||", ">>", "<<", ">=", "<=", "==", "!=", "2>"].includes(pair) ? start + 2 : start + 1;
}

function bareTokenEnd(characters: string[], start: number): number {
  let index = start;
  while (index < characters.length
    && !/\s/u.test(characters[index])
    && !operatorCharacters.has(characters[index])
    && !["'", '"', "`", "$"].includes(characters[index])) {
    index += 1;
  }
  return Math.max(start + 1, index);
}

function environmentAssignment(value: string): boolean {
  return /^[A-Za-z_][A-Za-z0-9_]*=.*$/u.test(value);
}

function classifyBareArgument(value: string): TerminalSemanticTokenKind | null {
  if (/^(?:=|==|!=|\+|-|\*|\/|\?|:)$/u.test(value)) return "operator";
  if (/^--?[A-Za-z0-9][\w-]*(?:=.*)?$/u.test(value)) return "option";
  if (ipv4Address(value) || ipv6Address(value) || /^\w[\w.-]*:\d{1,5}$/u.test(value)) return "address";
  if (/^(?:0x[\dA-Fa-f]+|[+-]?\d+(?:\.\d+)?(?:e[+-]?\d+)?%?)$/u.test(value)) return "number";
  if (/^(?:~|\.{1,2})?(?:[/\\][^\s]+)+$/u.test(value) || /^[A-Za-z]:\\[^\s]+$/u.test(value)) return "path";
  return null;
}

function ipv4Address(value: string): boolean {
  const parts = value.split(".");
  return parts.length === 4 && parts.every((part) => /^\d{1,3}$/u.test(part) && Number(part) <= 255);
}

function ipv6Address(value: string): boolean {
  return value.includes(":") && /^[\dA-Fa-f:]+$/u.test(value) && value.split(":").length >= 3;
}
