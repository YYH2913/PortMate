import { describe, expect, it, vi } from "vitest";
import type { IBufferLine } from "@xterm/xterm";
import { visibleSemanticLines } from "../../src/terminal-semantic-viewport";

function viewport(length: number, viewportY: number, lines: (index: number) => [boolean, string]) {
  return { length, viewportY, getLine: vi.fn((i: number) => {
    if (i < 0 || i >= length) return undefined;
    const [isWrapped, text] = lines(i);
    return { isWrapped, translateToString: () => text } as IBufferLine;
  }) };
}

describe("bounded normal-buffer highlighting", () => {
  it("joins wrap continuations at both viewport edges", () => {
    const buf = viewport(6, 1, i => [[1, 3, 4].includes(i), ["edge:~$ ", "ls", "hello", " w", "orld", "offscreen"][i]]);
    expect(visibleSemanticLines(buf, 3)).toEqual([
      { start: 0, end: 2, text: "edge:~$ ls" }, { start: 2, end: 5, text: "hello world" },
    ]);
  });
  it("never walks a huge wrapped line into deep history or below the viewport", () => {
    const huge = viewport(1_000_000, 400_000, i => [i !== 0 && i !== 400_005, i === 400_005 ? "WARN" : "x".repeat(80)]);
    expect(visibleSemanticLines(huge, 8)).toEqual([]); // both logical lines are oversized
    const indices = huge.getLine.mock.calls.map(([i]) => i);
    expect(Math.min(...indices)).toBeGreaterThanOrEqual(huge.viewportY - 64);
    expect(Math.max(...indices)).toBeLessThan(huge.viewportY + 8 + 64);
    const afterTrim = viewport(3, 0, i => [i < 2, i === 2 ? "OK" : "incomplete prefix"]);
    expect(visibleSemanticLines(afterTrim, 3)).toEqual([{ start: 2, end: 3, text: "OK" }]);
  });
});
