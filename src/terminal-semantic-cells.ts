export type TerminalSemanticCell = {
  row: number;
  column: number;
  width: number;
  colorable?: false;
};

export type TerminalSemanticBufferCell = {
  column: number;
  width: number;
  chars: string;
  colorable?: false;
};

export type TerminalSemanticMappedRow = {
  text: string;
  cells: TerminalSemanticCell[];
};

export function mapTerminalSemanticRow(
  row: number,
  bufferCells: readonly TerminalSemanticBufferCell[],
): TerminalSemanticMappedRow {
  const characters: string[] = [];
  const cells: TerminalSemanticCell[] = [];
  const content: boolean[] = [];

  for (const bufferCell of bufferCells) {
    if (bufferCell.width <= 0) continue;
    const cellCharacters = Array.from(bufferCell.chars || " ");
    for (const character of cellCharacters) {
      characters.push(character);
      const cell: TerminalSemanticCell = {
        row,
        column: bufferCell.column,
        width: Math.max(1, bufferCell.width),
      };
      if (bufferCell.colorable === false) cell.colorable = false;
      cells.push(cell);
      content.push(Boolean(bufferCell.chars));
    }
  }
  while (characters.length && !content.at(-1)) {
    characters.pop();
    cells.pop();
    content.pop();
  }
  return { text: characters.join(""), cells };
}

export function terminalSemanticCellSegments(
  cells: readonly TerminalSemanticCell[],
  start: number,
  end: number,
): TerminalSemanticCell[] {
  const segments: TerminalSemanticCell[] = [];
  for (const cell of cells.slice(Math.max(0, start), Math.max(0, end))) {
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
