export const TERMINAL_TEXT_EXPORT_REQUEST_EVENT = "portmate-terminal-text-export";
export const TERMINAL_TEXT_EXPORT_REQUEST_TIMEOUT_MS = 1_500;

export type TerminalTextExportSource = "buffer" | "selection";

export type TerminalTextExportPayload = {
  sessionId: string;
  viewId: string;
  source: TerminalTextExportSource;
  text: string;
  bytes: number;
  logicalLines: number;
};

export type TerminalTextExportResponse =
  | { ok: true; payload: TerminalTextExportPayload }
  | { ok: false; error: string };

export type TerminalTextExportRequestDetail = {
  sessionId: string;
  viewId: string;
  source: TerminalTextExportSource;
  respond: (response: TerminalTextExportResponse) => void;
};

export function requestTerminalTextExport(
  request: Omit<TerminalTextExportRequestDetail, "respond">,
  target: Pick<EventTarget, "dispatchEvent"> = window,
  timeoutMs = TERMINAL_TEXT_EXPORT_REQUEST_TIMEOUT_MS,
): Promise<TerminalTextExportPayload> {
  return new Promise((resolve, reject) => {
    let settled = false;
    const timeout = setTimeout(() => {
      if (settled) return;
      settled = true;
      reject(new Error("未找到目标终端视图。"));
    }, timeoutMs);
    const respond = (response: TerminalTextExportResponse) => {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      if (response.ok) resolve(response.payload);
      else reject(new Error(response.error));
    };
    target.dispatchEvent(new CustomEvent<TerminalTextExportRequestDetail>(TERMINAL_TEXT_EXPORT_REQUEST_EVENT, {
      detail: { ...request, respond },
    }));
  });
}
