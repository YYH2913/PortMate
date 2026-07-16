export const MAX_TERMINAL_EXPORT_BYTES = 16 * 1024 * 1024;

export type TerminalTextExtraction =
  | { ok: true; text: string; bytes: number; logicalLines: number }
  | { ok: false; reason: "empty" | "too-large"; bytes: number };

type TerminalBufferLineLike = {
  isWrapped?: boolean;
  translateToString(trimRight?: boolean): string;
};

type TerminalBufferLike = {
  length: number;
  getLine(index: number): TerminalBufferLineLike | undefined;
};

const textEncoder = new TextEncoder();

export function extractTerminalBufferText(
  buffer: TerminalBufferLike,
  maxBytes = MAX_TERMINAL_EXPORT_BYTES,
): TerminalTextExtraction {
  let lastContentRow = Math.max(-1, Math.trunc(buffer.length) - 1);
  while (lastContentRow >= 0 && !(buffer.getLine(lastContentRow)?.translateToString(true) ?? "")) {
    lastContentRow -= 1;
  }
  if (lastContentRow < 0) return { ok: false, reason: "empty", bytes: 0 };

  const chunks: string[] = [];
  let bytes = 0;
  let logicalLines = 0;
  for (let row = 0; row <= lastContentRow; row += 1) {
    const line = buffer.getLine(row);
    const prefix = row > 0 && !line?.isWrapped ? "\n" : "";
    const chunk = `${prefix}${line?.translateToString(true) ?? ""}`;
    bytes += textEncoder.encode(chunk).byteLength;
    if (bytes > maxBytes) return { ok: false, reason: "too-large", bytes };
    if (row === 0 || !line?.isWrapped) logicalLines += 1;
    chunks.push(chunk);
  }
  return { ok: true, text: chunks.join(""), bytes, logicalLines };
}

export function extractTerminalSelectionText(
  selection: string,
  maxBytes = MAX_TERMINAL_EXPORT_BYTES,
): TerminalTextExtraction {
  if (!selection) return { ok: false, reason: "empty", bytes: 0 };
  const bytes = textEncoder.encode(selection).byteLength;
  if (bytes > maxBytes) return { ok: false, reason: "too-large", bytes };
  return { ok: true, text: selection, bytes, logicalLines: selection.split("\n").length };
}
