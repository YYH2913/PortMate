import { describe, expect, it, vi } from "vitest";
import type { IBufferCell, Terminal } from "@xterm/xterm";
import { alternateTerminalScreenSnapshot } from "../../src/terminal-screen-snapshot";

const styles = ["isBold", "isItalic", "isDim", "isUnderline", "isBlink", "isInverse", "isInvisible", "isStrikethrough", "isOverline"] as const;
function cell(chars: string, patch: Partial<IBufferCell> = {}): IBufferCell {
  return {
    getChars: () => chars, getWidth: () => 1, getCode: () => chars.codePointAt(0) ?? 0,
    getFgColorMode: () => 0, getFgColor: () => -1, getBgColorMode: () => 0, getBgColor: () => -1,
    isAttributeDefault: () => !chars,
    ...Object.fromEntries(styles.map(name => [name, () => 0])), ...patch,
  } as IBufferCell;
}
function terminal(cells: IBufferCell[], wrapped = false) {
  const line = { length: cells.length, isWrapped: wrapped, getCell: (column: number) => cells[column] };
  return { cols: cells.length, rows: 1, buffer: { active: { type: "alternate", getNullCell: () => cell(""), getLine: () => line } } } as unknown as Terminal;
}
describe("compact alternate-screen snapshots", () => {
  it("distinguishes every style/color/code/width and wrapped state", () => {
    const base = alternateTerminalScreenSnapshot(terminal([cell("x")]));
    for (const style of styles) expect(alternateTerminalScreenSnapshot(terminal([cell("x", { [style]: () => 1 })]))).not.toEqual(base);
    for (const field of ["getFgColor", "getBgColor", "getFgColorMode", "getBgColorMode", "getWidth", "getCode"] as const) {
      expect(alternateTerminalScreenSnapshot(terminal([cell("x", { [field]: () => 2 })]))).not.toEqual(base);
    }
    expect(alternateTerminalScreenSnapshot(terminal([cell("x")], true))).not.toEqual(base);
    expect(alternateTerminalScreenSnapshot(terminal([cell("x"), cell("")]))).toEqual(base);
    expect(alternateTerminalScreenSnapshot(terminal([cell("x", { isUnderline: () => 1 })]))).not.toEqual(alternateTerminalScreenSnapshot(terminal([cell("x", { isUnderline: () => 0x08000000 })])));
    expect(alternateTerminalScreenSnapshot(terminal([cell(""), cell("x")]))).not.toEqual(base);
  });
  it("preserves exact Unicode and control text without JSON allocations", () => {
    const first = terminal([cell("界", { getWidth: () => 2 }), cell("", { getWidth: () => 0 }), cell("e\u0301🙂\0:")]);
    const json = vi.spyOn(JSON, "stringify");
    const before = json.mock.calls.length;
    const snapshot = alternateTerminalScreenSnapshot(first);
    const calls = json.mock.calls.length - before;
    json.mockRestore();
    expect(calls).toBe(0);
    expect(snapshot).toEqual(alternateTerminalScreenSnapshot(first));
    expect(snapshot).not.toEqual(alternateTerminalScreenSnapshot(terminal([cell("界"), cell("e\u0301🙂"), cell("\0:")])));
  });
});
