import { describe, expect, it, vi } from "vitest";
import { requestTerminalTextExport, TERMINAL_TEXT_EXPORT_REQUEST_EVENT } from "./terminal-export-event";
import type { TerminalTextExportRequestDetail } from "./terminal-export-event";

describe("terminal export request event", () => {
  it("resolves only through the matching terminal response callback", async () => {
    const dispatchEvent = vi.fn((event: Event) => {
      expect(event.type).toBe(TERMINAL_TEXT_EXPORT_REQUEST_EVENT);
      const detail = (event as CustomEvent<TerminalTextExportRequestDetail>).detail;
      expect(detail).toMatchObject({ sessionId: "session-a", viewId: "view-b", source: "buffer" });
      detail.respond({
        ok: true,
        payload: { sessionId: "session-a", viewId: "view-b", source: "buffer", text: "ready", bytes: 5, lineCount: 1 },
      });
      return true;
    });

    await expect(requestTerminalTextExport(
      { sessionId: "session-a", viewId: "view-b", source: "buffer" },
      { dispatchEvent },
      50,
    )).resolves.toMatchObject({ text: "ready", viewId: "view-b" });
    expect(dispatchEvent).toHaveBeenCalledOnce();
  });

  it("rejects explicit extraction errors and missing listeners", async () => {
    const rejectingTarget = {
      dispatchEvent(event: Event) {
        (event as CustomEvent<TerminalTextExportRequestDetail>).detail.respond({ ok: false, error: "empty" });
        return true;
      },
    };
    await expect(requestTerminalTextExport(
      { sessionId: "session-a", viewId: "view-a", source: "selection" },
      rejectingTarget,
      50,
    )).rejects.toThrow("empty");
    await expect(requestTerminalTextExport(
      { sessionId: "session-a", viewId: "missing", source: "buffer" },
      { dispatchEvent: () => true },
      1,
    )).rejects.toThrow("未找到目标终端视图");
  });
});
