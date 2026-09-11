import { describe, expect, it } from "vitest";
import { SerialColumnDetector } from "./terminal-serial-columns";

const shell = "\x1b[?2004h";
const redraw = (columns = 80) => "\r\x1b[K\x1b[A" + "\x1b[C".repeat(columns - 1) + "\x1b[K";
const bytes = (text: string) => new TextEncoder().encode(text);

describe("serial passive column detector", () => {
  it("requires two matching observations, without claiming an exact probability", () => {
    const d = new SerialColumnDetector();
    expect(d.observe(shell + redraw(), 0)).toBeNull();
    expect(d.observe(redraw(), 100)).toEqual({ columns: 80, observations: 2 });
    expect(d.observe(redraw().repeat(5), 101)).toEqual({ columns: 80, observations: 3 });
  });

  it("recognizes the fragmented Readline redraw from the COM4 incident", () => {
    const d = new SerialColumnDetector();
    const trace = shell + redraw() + "\r\n\r\x1b[K\x1b[A" + "\x1b[C".repeat(79) + "\b\x1b[K";
    let result;
    for (const byte of bytes(trace)) result = d.observe(Uint8Array.of(byte), 10);
    expect(result).toEqual({ columns: 80, observations: 2 });
  });

  it("is invariant under every possible two-part split of the stream", () => {
    const trace = bytes(shell + redraw().repeat(2));
    for (let i = 0; i <= trace.length; i++) {
      const d = new SerialColumnDetector();
      d.observe(trace.slice(0, i), 0);
      expect(d.observe(trace.slice(i), 1)?.columns).toBe(80);
    }
  });

  it("handles numeric CUF and omitted/zero default parameters", () => {
    const d = new SerialColumnDetector();
    expect(d.observe(shell + "\r\x1b[0K\x1b[1A\x1b[131C\x1b[K".repeat(2), 0)?.columns).toBe(132);
  });

  it("does not infer sizes from login prompts, ordinary logs or unmarked applications", () => {
    const d = new SerialColumnDetector();
    expect(d.observe("login: root\r\nPassword: " + redraw().repeat(3), 0)).toBeNull();
  });

  it.each([
    "\x1b[A\x1b[79C\x1b[K", // no known line start
    "\r\x1b[2A\x1b[79C\x1b[K", // more than one row
    "\r\x1b[Ahello\x1b[79C\x1b[K", // text interrupts sequence
    "\r\x1b[A\x1b[79C\n\x1b[K",
    "\r\x1b[A\x1b[?79C\x1b[K", // not ordinary CUF
    "\r\x1b[A\x1b[79;2C\x1b[K",
    "\r\x1b[A\x1b[80G\x1b[K", // absolute position alone is not width
    "\r\x1b[A\x1b[79C\x1b[2K", // not erase-to-end
    "\r\x1b[A\x1b[79C\x1b[H",
    "\r\x1b[A\x1b[79C\x1b[D",
  ])("rejects unrelated cursor sequences %j", sequence => {
    expect(new SerialColumnDetector().observe(shell + sequence.repeat(3), 0)).toBeNull();
  });

  it.each([47, 1047, 1049])("excludes alternate-screen mode %s and discards earlier evidence", mode => {
    const d = new SerialColumnDetector();
    d.observe(shell + redraw(), 0);
    expect(d.observe(`\x1b[?${mode}h` + redraw().repeat(3), 1)).toBeNull();
    expect(d.observe(`\x1b[?${mode}l` + redraw(), 2)).toBeNull();
    expect(d.observe(redraw(), 3)?.columns).toBe(80);
  });

  it.each(["]", "P", "_", "^", "X"])("ignores control-string payload %s without buffering it", prefix => {
    const d = new SerialColumnDetector();
    expect(d.observe(shell + `\x1b${prefix}` + redraw().repeat(50) + "\x1b\\", 0)).toBeNull();
    expect(d.observe(redraw(), 1)).toBeNull();
  });

  it("does not treat a BEL inside DCS as a terminator", () => {
    expect(new SerialColumnDetector().observe(shell + "\x1bP\x07" + redraw().repeat(3) + "\x1b\\", 0)).toBeNull();
  });

  it("expires evidence and prevents combining delayed partial redraws", () => {
    const d = new SerialColumnDetector();
    d.observe(shell + redraw(), 0);
    expect(d.observe(redraw(), 120_001)).toBeNull();
    d.observe("\r\x1b[A\x1b[79C", 120_002);
    expect(d.observe("\x1b[K", 123_003)).toBeNull();
  });

  it("restarts consensus after conflicting evidence", () => {
    const d = new SerialColumnDetector();
    d.observe(shell + redraw().repeat(2), 0);
    expect(d.observe(redraw(132), 1)).toBeNull();
    expect(d.observe(redraw(132), 2)?.columns).toBe(132);
  });

  it("clears state on reconnect/reset and terminal reset controls", () => {
    const d = new SerialColumnDetector();
    d.observe(shell + redraw().repeat(2), 0);
    d.reset();
    expect(d.observe(redraw().repeat(3), 1)).toBeNull();
    expect(d.observe(shell + redraw().repeat(2) + "\x1bc" + redraw().repeat(3), 2)).toBeNull();
    expect(d.observe(shell + redraw().repeat(2) + "\x1b[!p", 3)).toBeNull();
  });

  it("hides a previous suggestion while bracketed-paste shell mode is disabled", () => {
    const d = new SerialColumnDetector();
    expect(d.observe(shell + redraw().repeat(2) + "\x1b[?2004l", 0)).toBeNull();
  });

  it("bounds retained state and tolerates malformed CSI and cancellation", () => {
    const d = new SerialColumnDetector();
    expect(d.observe(shell + "\r\x1b[A\x1b[" + "9".repeat(100_000) + "C\x1b[K", 0)).toBeNull();
    expect(JSON.stringify(d).length).toBeLessThan(400);
    expect(d.observe("\r\x1b[A\x1b[79\x18C\x1b[K", 1)).toBeNull();
    expect(d.observe(redraw(), 2)).toBeNull();
  });

  it.each([19, 1025, 9999])("rejects out-of-range width %s", columns => {
    expect(new SerialColumnDetector().observe(shell + `\r\x1b[A\x1b[${columns - 1}C\x1b[K`.repeat(3), 0)).toBeNull();
  });
});
