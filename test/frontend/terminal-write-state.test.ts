import { describe, expect, it } from "vitest";
import { terminalWriteHasOutput } from "../../src/terminal-write-state";

describe("terminal visual write admission", () => {
  it("does not inspect or render outgoing captures and empty acknowledgements", () => {
    expect(terminalWriteHasOutput({ direction: "outbound", text: "x".repeat(65536) })).toBe(false);
    expect(terminalWriteHasOutput({ direction: "outbound", text: "\x1b[2J" }, new Uint8Array([27, 91, 50, 74]))).toBe(false);
    for (const direction of ["inbound", "system"] as const) {
      expect(terminalWriteHasOutput({ direction, text: "" })).toBe(false);
      expect(terminalWriteHasOutput({ direction, text: null })).toBe(false);
    }
  });
  it("keeps incoming binary/control output and gives raw bytes precedence over text", () => {
    expect(terminalWriteHasOutput({ direction: "inbound", text: "" }, new Uint8Array([0, 0x80, 255]))).toBe(true);
    expect(terminalWriteHasOutput({ direction: "inbound", text: "\x1b[?1049h" })).toBe(true);
    expect(terminalWriteHasOutput({ direction: "system", text: "diagnostic" })).toBe(true);
    expect(terminalWriteHasOutput({ direction: "inbound", text: "not actual raw output" }, new Uint8Array())).toBe(false);
  });
});
