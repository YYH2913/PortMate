export type TerminalMouseEncoding = "default" | "utf8" | "sgr" | "urxvt" | "sgr-pixels";

/**
 * XTerm's onBinary event uses a JavaScript string as a byte container. Keep
 * the conversion explicit so C1/high-coordinate bytes never get UTF-8
 * re-encoded while crossing the Tauri JSON boundary.
 */
export function terminalBinaryStringToBytes(value: string): number[] | null {
  const bytes = new Array<number>(value.length);
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code > 0xff) return null;
    bytes[index] = code;
  }
  return bytes;
}

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
