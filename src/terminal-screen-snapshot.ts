import type { Terminal } from "@xterm/xterm";

/** Exact row comparison, without per-cell arrays/JSON or a lossy hash. */
export function alternateTerminalScreenSnapshot(term: Pick<Terminal, "buffer" | "rows" | "cols">): string[] {
  const buffer = term.buffer.active;
  if (buffer.type !== "alternate") return [];
  const snapshot: string[] = [];
  const reusableCell = buffer.getNullCell();
  for (let row = 0; row < term.rows; row++) {
    const line = buffer.getLine(row);
    if (!line) { snapshot.push("missing"); continue; }
    let content = line.isWrapped ? "1" : "0";
    const columns = Math.min(term.cols, line.length);
    for (let column = 0; column < columns; column++) {
      const cell = line.getCell(column, reusableCell);
      if (!cell) continue;
      const chars = cell.getChars();
      const width = cell.getWidth();
      if (!chars && width === 1 && cell.isAttributeDefault()) continue;
      const code = cell.getCode();
      const fgMode = cell.getFgColorMode(), fg = cell.getFgColor();
      const bgMode = cell.getBgColorMode(), bg = cell.getBgColor();
      const underline = cell.isUnderline();
      const flags = (cell.isBold() ? 1 : 0) | (cell.isItalic() ? 2 : 0)
        | (cell.isDim() ? 4 : 0)
        | (cell.isBlink() ? 16 : 0) | (cell.isInverse() ? 32 : 0)
        | (cell.isInvisible() ? 64 : 0) | (cell.isStrikethrough() ? 128 : 0)
        | (cell.isOverline() ? 256 : 0);
      // Fixed-width fields followed by an explicit UTF-16 length make this
      // collision-free even for NULs, wide cells, combining text and emoji.
      content += String.fromCharCode(
        column >>> 16, column & 0xffff, width,
        code >>> 16, code & 0xffff, fgMode >>> 16, fgMode & 0xffff,
        fg >>> 16, fg & 0xffff, bgMode >>> 16, bgMode & 0xffff,
        bg >>> 16, bg & 0xffff, flags, underline >>> 16, underline & 0xffff,
        chars.length >>> 16, chars.length & 0xffff,
      ) + chars;
    }
    snapshot.push(content);
  }
  return snapshot;
}
