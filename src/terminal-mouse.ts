export type TerminalMouseEncoding = "default" | "utf8" | "sgr" | "urxvt" | "sgr-pixels";

export function isTerminalMouseReport(text: string): boolean {
  return /^\x1b\[<\d+;\d+;\d+[Mm]$/.test(text)
    || /^\x1b\[\d+;\d+;\d+M$/.test(text)
    || /^\x1b\[M[\s\S]{3,6}$/.test(text)
    || /^\x1b\[\d+;\d+;\d+;\d+&w$/.test(text);
}

export function reduceTerminalMouseEncoding(
  current: TerminalMouseEncoding,
  modes: readonly (number | number[])[],
  enabled: boolean,
): TerminalMouseEncoding {
  let next = current;
  for (const mode of modes.flatMap((value) => typeof value === "number" ? [value] : value)) {
    const encoding = terminalMouseEncodingFromMode(mode);
    if (!encoding) continue;
    if (enabled) next = encoding;
    else if (next === encoding) next = "default";
  }
  return next;
}

export function terminalMouseEncodingSequence(encoding: TerminalMouseEncoding): string {
  if (encoding === "utf8") return "\x1b[?1005h";
  if (encoding === "sgr") return "\x1b[?1006h";
  if (encoding === "urxvt") return "\x1b[?1015h";
  if (encoding === "sgr-pixels") return "\x1b[?1016h";
  return "";
}

function terminalMouseEncodingFromMode(mode: number): Exclude<TerminalMouseEncoding, "default"> | null {
  if (mode === 1005) return "utf8";
  if (mode === 1006) return "sgr";
  if (mode === 1015) return "urxvt";
  if (mode === 1016) return "sgr-pixels";
  return null;
}
