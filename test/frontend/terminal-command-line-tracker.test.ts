import { describe, expect, it, vi } from "vitest";
import { emptyTerminalCommandLineTracker, trackTerminalCommandInput } from "../../src/terminal-command-line-tracker";
import { MAX_COMMAND_HISTORY_COMMAND_CHARACTERS } from "../../src/command-history-state";

describe("terminal command line tracker", () => {
  it("only extracts terminated lines from a text payload", () => {
    expect(trackTerminalCommandInput(emptyTerminalCommandLineTracker(), "one\r\ntwo\npartial").submitted).toEqual(["one", "two"]);
    expect(trackTerminalCommandInput(emptyTerminalCommandLineTracker(), "unfinished").submitted).toEqual([]);
  });
  it("records a serial command edited with arrows, delete and backspace", () => {
    let state = emptyTerminalCommandLineTracker();
    state = trackTerminalCommandInput(state, "flash_emmc_overlayfs.sh").state;
    state = trackTerminalCommandInput(state, "\u001b[2D\u001b[3~").state;
    state = trackTerminalCommandInput(state, "-s\u001b[999D\u001b[999C").state;
    expect(trackTerminalCommandInput(state, "\r").submitted).toEqual(["flash_emmc_overlayfs.-sh"]);
  });

  it("does not record private/control lines", () => {
    const state = emptyTerminalCommandLineTracker();
    expect(trackTerminalCommandInput(state, "password\r", true).submitted).toEqual([]);
    expect(trackTerminalCommandInput(state, "\u001b[2J\r").submitted).toEqual([]);
  });

  it("deletes forward with the actual Delete CSI, including split input", () => {
    const state = trackTerminalCommandInput(emptyTerminalCommandLineTracker(), "abc\x1b[D").state;
    expect(trackTerminalCommandInput(state, "\x1b[3~\r").submitted).toEqual(["ab"]);
    const partial = trackTerminalCommandInput(state, "\x1b[3").state;
    expect(trackTerminalCommandInput(partial, "~\r").submitted).toEqual(["ab"]);
  });

  it("understands application-cursor SS3 edits without guessing remote history", () => {
    const start = trackTerminalCommandInput(emptyTerminalCommandLineTracker(), "abc\x1bO").state;
    expect(trackTerminalCommandInput(start, "D\x1b[3~\r").submitted).toEqual(["ab"]);
    expect(trackTerminalCommandInput(emptyTerminalCommandLineTracker(), "abc\x1bOA\r").submitted).toEqual([]);
  });

  it("edits non-BMP text as code points", () => {
    expect(trackTerminalCommandInput(emptyTerminalCommandLineTracker(), "a😀b\x1b[D\x7f\r").submitted).toEqual(["ab"]);
  });

  it("does not submit bracketed paste until a real Enter", () => {
    const pasted = trackTerminalCommandInput(emptyTerminalCommandLineTracker(), "\x1b[200~echo one\recho two\x1b[201~");
    expect(pasted.submitted).toEqual([]);
    expect(trackTerminalCommandInput(pasted.state, "\r").submitted).toEqual(["echo one\necho two"]);
  });

  it("does not expose a suffix of a private or unknown command", () => {
    const hidden = trackTerminalCommandInput(emptyTerminalCommandLineTracker(), "secret", true).state;
    expect(trackTerminalCommandInput(hidden, "-suffix\rpublic\r").submitted).toEqual(["public"]);
    expect(trackTerminalCommandInput(emptyTerminalCommandLineTracker(), "ls\t\r").submitted).toEqual([]);
  });

  it("handles backspace, multiple submissions and cancellation", () => {
    expect(trackTerminalCommandInput(emptyTerminalCommandLineTracker(), "git stats\x7fus\r\nprintf one\nprintf two\r")
      .submitted).toEqual(["git status", "printf one", "printf two"]);
    expect(trackTerminalCommandInput(emptyTerminalCommandLineTracker(), "cancelled\x03\r").submitted).toEqual([]);
    expect(trackTerminalCommandInput(emptyTerminalCommandLineTracker(), "secret\x1b[A\r").submitted).toEqual([]);
  });

  it("appends a long typed line without rebuilding its accumulated code-point array per key", () => {
    let state = emptyTerminalCommandLineTracker();
    const from = vi.spyOn(Array, "from");
    try {
      for (let index = 0; index < MAX_COMMAND_HISTORY_COMMAND_CHARACTERS; index++) {
        const previous = state;
        state = trackTerminalCommandInput(previous, "x").state;
        expect(previous.line.length).toBe(index); // immutable snapshots
      }
      expect(from).not.toHaveBeenCalled();
    } finally { from.mockRestore(); }
    expect(trackTerminalCommandInput(state, "\r").submitted).toEqual(["x".repeat(MAX_COMMAND_HISTORY_COMMAND_CHARACTERS)]);
    const overflow = trackTerminalCommandInput(state, "x").state;
    expect(overflow.synchronized).toBe(false);
    expect(overflow.line).toBe("");
    expect(trackTerminalCommandInput(overflow, "suffix\rpublic\r").submitted).toEqual(["public"]);
  });

  it("preserves code-point edits and bracketed paste across the append fast path", () => {
    let state = trackTerminalCommandInput(emptyTerminalCommandLineTracker(), "ab").state;
    state = trackTerminalCommandInput(state, "中😀").state;
    expect(state.cursor).toBe(4);
    state = trackTerminalCommandInput(state, "c\x1b[D\x7f").state;
    expect(trackTerminalCommandInput(state, "\r").submitted).toEqual(["ab中c"]);
    state = trackTerminalCommandInput(emptyTerminalCommandLineTracker(), "\x1b[200~").state;
    state = trackTerminalCommandInput(state, "echo one").state;
    state = trackTerminalCommandInput(state, "\recho two\x1b[201~").state;
    expect(trackTerminalCommandInput(state, "\r").submitted).toEqual(["echo one\necho two"]);
  });

  it("deletes at a BMP line end without copying, but keeps literal paste controls conservative", () => {
    let state = trackTerminalCommandInput(emptyTerminalCommandLineTracker(), "中ab").state;
    const from = vi.spyOn(Array, "from");
    try {
      for (const text of ["\b", "\x7f", "\b", "\x7f"]) state = trackTerminalCommandInput(state, text).state;
      expect(from).not.toHaveBeenCalled();
      expect(state).toEqual(emptyTerminalCommandLineTracker());
    } finally { from.mockRestore(); }
    state = trackTerminalCommandInput(emptyTerminalCommandLineTracker(), "\x1b[200~abc").state;
    state = trackTerminalCommandInput(state, "\x7f").state;
    expect(state.synchronized).toBe(false);
    expect(state.line).toBe("");
    expect(trackTerminalCommandInput(state, "\x1b[201~\r").submitted).toEqual([]);
  });
});
