import { describe, expect, it } from "vitest";
import {
  appendTerminalSemanticRow,
  terminalSemanticCellSegments,
} from "../../src/terminal-semantic-cells";
import type { TerminalSemanticCell } from "../../src/terminal-semantic-cells";

type FixtureCell = { column: number; width: number; chars: string; colorable?: false };

function readRow(row: number, text: string, fixture: FixtureCell[], cols = 120) {
  const byColumn = new Map(fixture.map(c => [c.column, c]));
  const cells: TerminalSemanticCell[] = [];
  const reads: number[] = [];
  appendTerminalSemanticRow(cells, row, cols, text.length, column => {
    reads.push(column);
    const c = byColumn.get(column);
    return { getWidth: () => c?.width ?? 1, getChars: () => c?.chars ?? "", isAttributeDefault: () => c?.colorable !== false };
  });
  return { cells, reads };
}

function asciiCells(text: string, startColumn = 0): FixtureCell[] {
  return Array.from(text, (chars, index) => ({ column: startColumn + index, width: 1, chars }));
}

describe("terminal semantic cell mapping", () => {
  it("maps ASCII code-point ranges directly to terminal columns", () => {
    const mapped = readRow(3, "root# echo", asciiCells("root# echo"));
    expect(terminalSemanticCellSegments(mapped.cells, 6, 10)).toEqual([
      { row: 3, column: 6, width: 4 },
    ]);
  });

  it("accounts for CJK wide cells before and inside a token", () => {
    const mapped = readRow(4, "设备# /tmp/固件.bin", [
      { column: 0, width: 2, chars: "设" },
      { column: 1, width: 0, chars: "" },
      { column: 2, width: 2, chars: "备" },
      { column: 3, width: 0, chars: "" },
      ...asciiCells("# /tmp/", 4),
      { column: 11, width: 2, chars: "固" },
      { column: 12, width: 0, chars: "" },
      { column: 13, width: 2, chars: "件" },
      { column: 14, width: 0, chars: "" },
      ...asciiCells(".bin", 15),
    ]);
    expect(terminalSemanticCellSegments(mapped.cells, 4, 15)).toEqual([
      { row: 4, column: 6, width: 13 },
    ]);
  });

  it("maps emoji sequences to their owning wide cell", () => {
    const mapped = readRow(5, "👩‍💻# echo", [
      { column: 0, width: 2, chars: "👩‍💻" },
      { column: 1, width: 0, chars: "" },
      ...asciiCells("# echo", 2),
    ]);
    expect(terminalSemanticCellSegments(mapped.cells, 0, 3)).toEqual([
      { row: 5, column: 0, width: 2 },
    ]);
    expect(terminalSemanticCellSegments(mapped.cells, 5, 9)).toEqual([
      { row: 5, column: 4, width: 4 },
    ]);
  });

  it("keeps combining marks attached to the base cell", () => {
    const mapped = readRow(6, "e\u0301cho", [
      { column: 0, width: 1, chars: "e\u0301" },
      ...asciiCells("cho", 1),
    ]);
    expect(terminalSemanticCellSegments(mapped.cells, 0, 5)).toEqual([
      { row: 6, column: 0, width: 4 },
    ]);
    expect(terminalSemanticCellSegments(mapped.cells, 1, 2)).toEqual([
      { row: 6, column: 0, width: 1 },
    ]);
  });

  it("ignores width-zero continuation cells and trims only empty padding", () => {
    const mapped = readRow(7, "中 x", [
      { column: 0, width: 2, chars: "中" },
      { column: 1, width: 0, chars: "unexpected" },
      { column: 2, width: 1, chars: " " },
      { column: 3, width: 1, chars: "x" },
      { column: 4, width: 1, chars: "" },
      { column: 5, width: 1, chars: "" },
    ]);
    expect(mapped.reads).toEqual([0, 2, 3]); // neither the wide continuation nor the 116 padded cells
    expect(mapped.cells).toEqual([
      { row: 7, column: 0, width: 2 },
      { row: 7, column: 2, width: 1 },
      { row: 7, column: 3, width: 1 },
    ]);
  });

  it("counts emoji code points without scanning trailing cells", () => {
    const mapped = readRow(7, "👩‍💻x", [
      { column: 0, width: 2, chars: "👩‍💻" },
      { column: 1, width: 0, chars: "" },
      { column: 2, width: 1, chars: "x" },
    ]);
    expect(mapped.cells).toHaveLength(4);
    expect(mapped.reads).toEqual([0, 2]);
  });

  it("splits a token into one decoration per wrapped physical row", () => {
    const cells = [
      ...readRow(8, "       abc", asciiCells("abc", 7)).cells,
      ...readRow(9, "def", asciiCells("def", 0)).cells,
    ];
    expect(terminalSemanticCellSegments(cells, 8, 12)).toEqual([
      { row: 8, column: 8, width: 2 },
      { row: 9, column: 0, width: 2 },
    ]);
  });

  it("does not decorate cells that already carry an ANSI foreground", () => {
    const mapped = readRow(10, "ERROR 404 failed", [
      ...asciiCells("ERROR "),
      { column: 6, width: 1, chars: "4", colorable: false },
      { column: 7, width: 1, chars: "0", colorable: false },
      { column: 8, width: 1, chars: "4", colorable: false },
      ...asciiCells(" failed", 9),
    ]);

    expect(terminalSemanticCellSegments(mapped.cells, 0, mapped.cells.length)).toEqual([
      { row: 10, column: 0, width: 6 },
      { row: 10, column: 9, width: 7 },
    ]);
  });
});
