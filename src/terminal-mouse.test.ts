import { describe, expect, it } from "vitest";
import { isTerminalMouseReport, reduceTerminalMouseEncoding, terminalMouseEncodingSequence } from "./terminal-mouse";

describe("terminal mouse reporting", () => {
  it("recognizes complete SGR, pixel, URXVT, X10 and locator reports", () => {
    expect(isTerminalMouseReport("\x1b[<0;12;8M")).toBe(true);
    expect(isTerminalMouseReport("\x1b[<0;120;80m")).toBe(true);
    expect(isTerminalMouseReport("\x1b[32;12;8M")).toBe(true);
    expect(isTerminalMouseReport("\x1b[M !!")).toBe(true);
    expect(isTerminalMouseReport("\x1b[2;0;12;8&w")).toBe(true);
  });

  it("does not consume keyboard CSI, focus or fragmented reports", () => {
    expect(isTerminalMouseReport("\x1b[A")).toBe(false);
    expect(isTerminalMouseReport("\x1b[I")).toBe(false);
    expect(isTerminalMouseReport("\x1b[<0;12;8")).toBe(false);
    expect(isTerminalMouseReport("x\x1b[<0;12;8M")).toBe(false);
    expect(isTerminalMouseReport("\x1b[<0;12;8Mx")).toBe(false);
  });

  it("tracks DEC mouse encodings for terminal cache restoration", () => {
    expect(reduceTerminalMouseEncoding("default", [1000, 1006], true)).toBe("sgr");
    expect(reduceTerminalMouseEncoding("sgr", [[1005, 1016]], true)).toBe("sgr-pixels");
    expect(reduceTerminalMouseEncoding("sgr-pixels", [1006], false)).toBe("sgr-pixels");
    expect(reduceTerminalMouseEncoding("sgr-pixels", [1016], false)).toBe("default");
    expect(terminalMouseEncodingSequence("default")).toBe("");
    expect(terminalMouseEncodingSequence("utf8")).toBe("\x1b[?1005h");
    expect(terminalMouseEncodingSequence("sgr")).toBe("\x1b[?1006h");
    expect(terminalMouseEncodingSequence("urxvt")).toBe("\x1b[?1015h");
    expect(terminalMouseEncodingSequence("sgr-pixels")).toBe("\x1b[?1016h");
  });
});
