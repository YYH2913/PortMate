import type { IBuffer } from "@xterm/xterm";
import { MAX_TERMINAL_SEMANTIC_LINE_CHARACTERS } from "./terminal-semantic-highlighting";
import { appendTerminalSemanticRow } from "./terminal-semantic-cells";
import type { TerminalSemanticCell } from "./terminal-semantic-cells";

// Bound both ends of a wrapped logical line, even if a multi-megabyte paste
// contains nothing but padding. Never walk to the start/end of scrollback.
const MAX_WRAP_ROWS = 64;
export function visibleSemanticLines(buffer: Pick<IBuffer, "getLine" | "length" | "viewportY">, rows: number) {
  const lines: Array<{ start: number; end: number; text: string }> = [];
  const viewportEnd = Math.min(buffer.length, buffer.viewportY + rows);
  let row = buffer.viewportY;
  const lower = Math.max(0, row - MAX_WRAP_ROWS + 1);
  while (row > lower && buffer.getLine(row)?.isWrapped) row--;
  while (row < viewportEnd) {
    const start = row;
    const head = buffer.getLine(row);
    // An unseen prefix could change the meaning of every visible token.
    let skip = !head || head.isWrapped;
    let text = "";
    do {
      if (skip && row >= viewportEnd) break;
      if (!skip) {
        if (row - start >= MAX_WRAP_ROWS) skip = true;
        else {
          text += buffer.getLine(row)?.translateToString(true) ?? "";
          if (text.length > MAX_TERMINAL_SEMANTIC_LINE_CHARACTERS * 2) skip = true;
        }
      }
      row++;
    } while (row < buffer.length && buffer.getLine(row)?.isWrapped);
    if (!skip && text) lines.push({ start, end: row, text });
  }
  return lines;
}

/** Read attributes/geometry only for lexical matches, never for an entire blank or prose-only viewport. */
export function semanticLineCells(buffer: IBuffer, start: number, end: number, cols: number, textUnits: number): TerminalSemanticCell[] {
  const scratch = buffer.getNullCell();
  const cells: TerminalSemanticCell[] = [];
  for (let row = start; row < end; row++) {
    const line = buffer.getLine(row);
    if (!line) continue;
    // The common unwrapped case already has the trimmed length from the text-only pass.
    const rowUnits = end === start + 1 ? textUnits : line.translateToString(true).length;
    appendTerminalSemanticRow(cells, row, Math.min(cols, line.length), rowUnits, col => line.getCell(col, scratch));
  }
  return cells;
}
