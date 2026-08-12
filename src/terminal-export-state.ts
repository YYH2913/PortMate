import { normalizeTerminalTimestamps } from "./terminal-timestamp-state";
import type { TerminalTimestampEntry } from "./terminal-timestamp-state";

export const MAX_TERMINAL_EXPORT_BYTES = 16 * 1024 * 1024;

export type TerminalTextExtraction =
  | { ok: true; text: string; bytes: number; lineCount: number }
  | { ok: false; reason: "empty" | "missing-timestamp" | "too-large"; bytes: number };

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
  timestamps: readonly TerminalTimestampEntry[],
  maxBytes = MAX_TERMINAL_EXPORT_BYTES,
): TerminalTextExtraction {
  let lastContentRow = Math.max(-1, Math.trunc(buffer.length) - 1);
  while (lastContentRow >= 0 && !(buffer.getLine(lastContentRow)?.translateToString(true) ?? "")) {
    lastContentRow -= 1;
  }
  if (lastContentRow < 0) return { ok: false, reason: "empty", bytes: 0 };

  const normalizedTimestamps = normalizeTerminalTimestamps(
    timestamps,
    Math.max(1, timestamps.length),
  );
  const chunks: string[] = [];
  let bytes = 0;
  let timestampIndex = -1;
  for (let row = 0; row <= lastContentRow; row += 1) {
    const line = buffer.getLine(row);
    while (timestampIndex + 1 < normalizedTimestamps.length
      && normalizedTimestamps[timestampIndex + 1].line <= row) timestampIndex += 1;
    const timestamp = normalizedTimestamps[timestampIndex]?.ts;
    if (!timestamp) return { ok: false, reason: "missing-timestamp", bytes };
    const chunk = `${row > 0 ? "\n" : ""}[${timestamp}] ${line?.translateToString(true) ?? ""}`;
    bytes += textEncoder.encode(chunk).byteLength;
    if (bytes > maxBytes) return { ok: false, reason: "too-large", bytes };
    chunks.push(chunk);
  }
  return { ok: true, text: chunks.join(""), bytes, lineCount: lastContentRow + 1 };
}

export function extractTerminalSelectionText(
  selection: string,
  maxBytes = MAX_TERMINAL_EXPORT_BYTES,
): TerminalTextExtraction {
  if (!selection) return { ok: false, reason: "empty", bytes: 0 };
  const bytes = textEncoder.encode(selection).byteLength;
  if (bytes > maxBytes) return { ok: false, reason: "too-large", bytes };
  return { ok: true, text: selection, bytes, lineCount: selection.split("\n").length };
}
