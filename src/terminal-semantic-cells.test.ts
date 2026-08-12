import { describe, expect, it } from "vitest";
import {
  mapTerminalSemanticRow,
  terminalSemanticCellSegments,
} from "./terminal-semantic-cells";
import type { TerminalSemanticBufferCell } from "./terminal-semantic-cells";

function asciiCells(text: string, startColumn = 0): TerminalSemanticBufferCell[] {
  return Array.from(text, (chars, index) => ({ column: startColumn + index, width: 1, chars }));
}

describe("terminal semantic cell mapping", () => {
  it("maps ASCII code-point ranges directly to terminal columns", () => {
    const mapped = mapTerminalSemanticRow(3, asciiCells("root# echo"));
    expect(mapped.text).toBe("root# echo");
    expect(terminalSemanticCellSegments(mapped.cells, 6, 10)).toEqual([
      { row: 3, column: 6, width: 4 },
    ]);
  });

  it("accounts for CJK wide cells before and inside a token", () => {
    const mapped = mapTerminalSemanticRow(4, [
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
    expect(mapped.text).toBe("设备# /tmp/固件.bin");
    expect(terminalSemanticCellSegments(mapped.cells, 4, 15)).toEqual([
      { row: 4, column: 6, width: 13 },
    ]);
  });

  it("maps emoji sequences to their owning wide cell", () => {
    const mapped = mapTerminalSemanticRow(5, [
      { column: 0, width: 2, chars: "👩‍💻" },
      { column: 1, width: 0, chars: "" },
      ...asciiCells("# echo", 2),
    ]);
    expect(mapped.text).toBe("👩‍💻# echo");
    expect(terminalSemanticCellSegments(mapped.cells, 0, 3)).toEqual([
      { row: 5, column: 0, width: 2 },
    ]);
    expect(terminalSemanticCellSegments(mapped.cells, 5, 9)).toEqual([
      { row: 5, column: 4, width: 4 },
    ]);
  });

  it("keeps combining marks attached to the base cell", () => {
    const mapped = mapTerminalSemanticRow(6, [
      { column: 0, width: 1, chars: "e\u0301" },
      ...asciiCells("cho", 1),
    ]);
    expect(mapped.text).toBe("e\u0301cho");
    expect(terminalSemanticCellSegments(mapped.cells, 0, 5)).toEqual([
      { row: 6, column: 0, width: 4 },
    ]);
    expect(terminalSemanticCellSegments(mapped.cells, 1, 2)).toEqual([
      { row: 6, column: 0, width: 1 },
    ]);
  });

  it("ignores width-zero continuation cells and trims only empty padding", () => {
    const mapped = mapTerminalSemanticRow(7, [
      { column: 0, width: 2, chars: "中" },
      { column: 1, width: 0, chars: "unexpected" },
      { column: 2, width: 1, chars: " " },
      { column: 3, width: 1, chars: "x" },
      { column: 4, width: 1, chars: "" },
      { column: 5, width: 1, chars: "" },
    ]);
    expect(mapped.text).toBe("中 x");
    expect(mapped.cells).toEqual([
      { row: 7, column: 0, width: 2 },
      { row: 7, column: 2, width: 1 },
      { row: 7, column: 3, width: 1 },
    ]);
  });

  it("splits a token into one decoration per wrapped physical row", () => {
    const cells = [
      ...mapTerminalSemanticRow(8, asciiCells("abc", 7)).cells,
      ...mapTerminalSemanticRow(9, asciiCells("def", 0)).cells,
    ];
    expect(terminalSemanticCellSegments(cells, 1, 5)).toEqual([
      { row: 8, column: 8, width: 2 },
      { row: 9, column: 0, width: 2 },
    ]);
  });
});
