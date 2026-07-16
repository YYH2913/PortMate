import { describe, expect, it, vi } from "vitest";
import {
  executeTerminalSelectionAction,
  requestTerminalSelection,
  terminalBlockSelectionMouseEventInit,
  terminalSelectionShortcut,
  TERMINAL_SELECTION_REQUEST_EVENT,
} from "./terminal-selection-event";
import type { TerminalSelectionRequestDetail } from "./terminal-selection-event";

describe("terminal selection events", () => {
  it("resolves through the exact terminal response callback", async () => {
    const dispatchEvent = vi.fn((event: Event) => {
      expect(event.type).toBe(TERMINAL_SELECTION_REQUEST_EVENT);
      const detail = (event as CustomEvent<TerminalSelectionRequestDetail>).detail;
      expect(detail).toMatchObject({ sessionId: "session-a", viewId: "view-b", action: "copy" });
      detail.respond({
        ok: true,
        payload: { sessionId: "session-a", viewId: "view-b", action: "copy", selection: "selected" },
      });
      return true;
    });

    await expect(requestTerminalSelection(
      { sessionId: "session-a", viewId: "view-b", action: "copy" },
      { dispatchEvent },
      50,
    )).resolves.toMatchObject({ viewId: "view-b", selection: "selected" });
    expect(dispatchEvent).toHaveBeenCalledOnce();
  });

  it("rejects explicit errors and missing focused views", async () => {
    await expect(requestTerminalSelection(
      { sessionId: "session-a", viewId: "view-a", action: "copy" },
      {
        dispatchEvent(event: Event) {
          (event as CustomEvent<TerminalSelectionRequestDetail>).detail.respond({ ok: false, error: "empty" });
          return true;
        },
      },
      50,
    )).rejects.toThrow("empty");
    await expect(requestTerminalSelection(
      { sessionId: "session-a", viewId: "missing", action: "clear" },
      { dispatchEvent: () => true },
      1,
    )).rejects.toThrow("未找到目标终端视图");
  });

  it("validates the response target and copies only explicit copy results", async () => {
    const clipboard = { writeText: vi.fn().mockResolvedValue(undefined) };
    const target = {
      dispatchEvent(event: Event) {
        const detail = (event as CustomEvent<TerminalSelectionRequestDetail>).detail;
        detail.respond({
          ok: true,
          payload: { sessionId: detail.sessionId, viewId: detail.viewId, action: detail.action, selection: "exact" },
        });
        return true;
      },
    };
    await expect(executeTerminalSelectionAction(
      { sessionId: "session-a", viewId: "view-a", action: "copy" },
      target,
      clipboard,
    )).resolves.toMatchObject({ selection: "exact" });
    expect(clipboard.writeText).toHaveBeenCalledWith("exact");

    const mismatched = {
      dispatchEvent(event: Event) {
        const detail = (event as CustomEvent<TerminalSelectionRequestDetail>).detail;
        detail.respond({
          ok: true,
          payload: { sessionId: detail.sessionId, viewId: "wrong", action: detail.action, selection: null },
        });
        return true;
      },
    };
    await expect(executeTerminalSelectionAction(
      { sessionId: "session-a", viewId: "view-a", action: "clear" },
      mismatched,
      clipboard,
    )).rejects.toThrow("目标视图不匹配");
  });

  it("matches WindTerm copy and select-all shortcuts only in Remote and Local modes", () => {
    const shortcut = (key: string, mode: "remote" | "local" | "normal" | "command") => terminalSelectionShortcut({
      altKey: false,
      ctrlKey: true,
      key,
      metaKey: false,
      shiftKey: true,
    }, mode);
    expect(shortcut("C", "remote")).toBe("copy");
    expect(shortcut("a", "local")).toBe("select-all");
    expect(shortcut("c", "normal")).toBeNull();
    expect(shortcut("a", "command")).toBeNull();
    expect(terminalSelectionShortcut({ altKey: true, ctrlKey: true, key: "a", metaKey: false, shiftKey: true }, "remote")).toBeNull();
  });

  it("converts block-selection mouse downs to column selection and force-selects over mouse reporting", () => {
    const mouse = { button: 0, buttons: 1, clientX: 12, clientY: 34, ctrlKey: false, metaKey: false, screenX: 56, screenY: 78 };
    expect(terminalBlockSelectionMouseEventInit(mouse, false)).toMatchObject({
      altKey: true,
      shiftKey: false,
      detail: 1,
      clientX: 12,
      clientY: 34,
    });
    expect(terminalBlockSelectionMouseEventInit(mouse, true)).toMatchObject({ altKey: true, shiftKey: true });
  });
});
