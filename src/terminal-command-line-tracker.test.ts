import { describe, expect, it } from "vitest";
import { emptyTerminalCommandLineTracker, trackTerminalCommandInput } from "./terminal-command-line-tracker";

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
});
