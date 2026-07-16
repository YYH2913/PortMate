import { describe, expect, it } from "vitest";
import {
  executeTerminalBufferAction,
  requestTerminalBufferAction,
  resolveTerminalBufferAction,
  terminalBufferShortcut,
  TERMINAL_BUFFER_ACTION_REQUEST_EVENT,
} from "./terminal-buffer-event";
import type { TerminalBufferActionRequestDetail } from "./terminal-buffer-event";

describe("terminal buffer actions", () => {
  it("uses distinct local control sequences and protects alternate screen", () => {
    expect(resolveTerminalBufferAction("clear-scrollback", "normal")).toEqual({ ok: true, sequence: "\u001b[3J" });
    expect(resolveTerminalBufferAction("clear-screen", "normal")).toEqual({ ok: true, sequence: "\u001b[2J\u001b[H" });
    expect(resolveTerminalBufferAction("clear-all", "normal")).toEqual({ ok: true, sequence: "\u001b[2J\u001b[3J\u001b[H" });
    expect(resolveTerminalBufferAction("clear-screen", "alternate")).toEqual({
      ok: false,
      error: "全屏程序使用 alternate screen 时不能清除当前屏幕。",
    });
    expect(resolveTerminalBufferAction("clear-scrollback", "alternate")).toEqual({ ok: true, sequence: "\u001b[3J" });
  });

  it("matches WindTerm clear shortcuts only in Remote and Local modes", () => {
    const input = { altKey: false, ctrlKey: true, key: "l", metaKey: false, shiftKey: false };
    expect(terminalBufferShortcut(input, "remote", false)).toBe("clear-screen");
    expect(terminalBufferShortcut({ ...input, shiftKey: true }, "local", false)).toBe("clear-scrollback");
    expect(terminalBufferShortcut(input, "normal", false)).toBeNull();
    expect(terminalBufferShortcut(input, "command", false)).toBeNull();
    expect(terminalBufferShortcut({ ...input, ctrlKey: false, metaKey: true }, "remote", true)).toBe("clear-screen");
    expect(terminalBufferShortcut(input, "remote", true)).toBeNull();
  });

  it("resolves through the matching target response", async () => {
    const target = {
      dispatchEvent(event: Event) {
        expect(event.type).toBe(TERMINAL_BUFFER_ACTION_REQUEST_EVENT);
        const detail = (event as CustomEvent<TerminalBufferActionRequestDetail>).detail;
        detail.respond({
          ok: true,
          payload: { sessionId: detail.sessionId, viewId: detail.viewId, action: detail.action, bufferType: "normal" },
        });
        return true;
      },
    };
    await expect(executeTerminalBufferAction(
      { sessionId: "session-a", viewId: "view-b", action: "clear-screen" },
      target,
    )).resolves.toMatchObject({ viewId: "view-b", action: "clear-screen", bufferType: "normal" });
  });

  it("rejects explicit errors, target mismatches, and missing views", async () => {
    const rejecting = {
      dispatchEvent(event: Event) {
        (event as CustomEvent<TerminalBufferActionRequestDetail>).detail.respond({ ok: false, error: "alternate" });
        return true;
      },
    };
    await expect(requestTerminalBufferAction(
      { sessionId: "session-a", viewId: "view-a", action: "clear-all" },
      rejecting,
      50,
    )).rejects.toThrow("alternate");

    const mismatched = {
      dispatchEvent(event: Event) {
        const detail = (event as CustomEvent<TerminalBufferActionRequestDetail>).detail;
        detail.respond({
          ok: true,
          payload: { sessionId: detail.sessionId, viewId: "wrong", action: detail.action, bufferType: "normal" },
        });
        return true;
      },
    };
    await expect(executeTerminalBufferAction(
      { sessionId: "session-a", viewId: "view-a", action: "clear-scrollback" },
      mismatched,
    )).rejects.toThrow("目标视图不匹配");
    await expect(requestTerminalBufferAction(
      { sessionId: "session-a", viewId: "missing", action: "clear-screen" },
      { dispatchEvent: () => true },
      1,
    )).rejects.toThrow("未找到目标终端视图");
  });
});
