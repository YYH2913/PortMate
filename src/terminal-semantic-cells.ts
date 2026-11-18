import type { IBufferCell } from "@xterm/xterm";

export type TerminalSemanticCell = {
  row: number;
  column: number;
  width: number;
  colorable?: false;
};

type SemanticCellReader = (column: number) => Pick<IBufferCell, "getWidth" | "getChars" | "isAttributeDefault"> | undefined;

/**
 * Map code-point offsets to cells. The trimmed UTF-16 length from translateToString
 * bounds reads; each code point of a combined glyph maps to the same cell.
 */
export function appendTerminalSemanticRow(
  cells: TerminalSemanticCell[],
  row: number,
  columns: number,
  textUnits: number,
  read: SemanticCellReader,
) {
  let consumedUnits = 0;
  for (let column = 0; column < columns && consumedUnits < textUnits;) {
    const cell = read(column);
    const width = cell?.getWidth() ?? 0;
    if (!cell || width === 0) {
      column += 1;
      continue;
    }
    const chars = cell.getChars() || " "; // interior empty cells are spaces in translateToString
    const position: TerminalSemanticCell = { row, column, width };
    // Foreground, background, inverse, faint and concealed attributes all belong to the remote program.
    if (!cell.isAttributeDefault()) position.colorable = false;
    if (chars.length === 1) cells.push(position);
    else for (const _ of chars) cells.push(position);
    consumedUnits += chars.length;
    column += width;
  }
}

export function terminalSemanticCellSegments(
  cells: readonly TerminalSemanticCell[],
  start: number,
  end: number,
): TerminalSemanticCell[] {
  const segments: TerminalSemanticCell[] = [];
  const stop = Math.min(end, cells.length);
  for (let i = Math.max(0, start); i < stop; i++) {
    const cell = cells[i];
    if (cell.colorable === false) continue;
    const previous = segments.at(-1);
    if (previous && previous.row === cell.row && cell.column <= previous.column + previous.width) {
      previous.width = Math.max(previous.width, cell.column + cell.width - previous.column);
    } else {
      segments.push({ ...cell });
    }
  }
  return segments;
}
