import type { TerminalKeyMode } from "./terminal-key-mode";

export const TERMINAL_BUFFER_ACTION_REQUEST_EVENT = "portmate-terminal-buffer-action";
export const TERMINAL_BUFFER_ACTION_TIMEOUT_MS = 1_500;

export type TerminalBufferAction = "clear-scrollback" | "clear-screen" | "clear-all";
export type TerminalBufferType = "normal" | "alternate";

export type TerminalBufferActionPayload = {
  sessionId: string;
  viewId: string;
  action: TerminalBufferAction;
  bufferType: TerminalBufferType;
};

export type TerminalBufferActionResponse =
  | { ok: true; payload: TerminalBufferActionPayload }
  | { ok: false; error: string };

export type TerminalBufferActionRequestDetail = {
  sessionId: string;
  viewId: string;
  action: TerminalBufferAction;
  respond: (response: TerminalBufferActionResponse) => void;
};

type KeyboardShortcutLike = Pick<KeyboardEvent, "altKey" | "ctrlKey" | "key" | "metaKey" | "shiftKey">;

export type TerminalBufferActionResolution =
  | { ok: true; sequence: string }
  | { ok: false; error: string };

export function resolveTerminalBufferAction(
  action: TerminalBufferAction,
  bufferType: TerminalBufferType,
): TerminalBufferActionResolution {
  if (bufferType === "alternate" && action !== "clear-scrollback") {
    return { ok: false, error: "全屏程序使用 alternate screen 时不能清除当前屏幕。" };
  }
  switch (action) {
    case "clear-scrollback":
      return { ok: true, sequence: "\u001b[3J" };
    case "clear-screen":
      return { ok: true, sequence: "\u001b[2J\u001b[H" };
    case "clear-all":
      return { ok: true, sequence: "\u001b[2J\u001b[3J\u001b[H" };
  }
}

export function terminalBufferShortcut(
  event: KeyboardShortcutLike,
  mode: TerminalKeyMode,
  isMac = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform),
): "clear-screen" | "clear-scrollback" | null {
  if ((mode !== "remote" && mode !== "local") || event.altKey || event.key.toLowerCase() !== "l") return null;
  const primary = isMac
    ? event.metaKey && !event.ctrlKey
    : event.ctrlKey && !event.metaKey;
  if (!primary) return null;
  return event.shiftKey ? "clear-scrollback" : "clear-screen";
}

export function requestTerminalBufferAction(
  request: Omit<TerminalBufferActionRequestDetail, "respond">,
  target: Pick<EventTarget, "dispatchEvent"> = window,
  timeoutMs = TERMINAL_BUFFER_ACTION_TIMEOUT_MS,
): Promise<TerminalBufferActionPayload> {
  return new Promise((resolve, reject) => {
    let settled = false;
    const timeout = setTimeout(() => {
      if (settled) return;
      settled = true;
      reject(new Error("未找到目标终端视图。"));
    }, timeoutMs);
    const respond = (response: TerminalBufferActionResponse) => {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      if (response.ok) resolve(response.payload);
      else reject(new Error(response.error));
    };
    target.dispatchEvent(new CustomEvent<TerminalBufferActionRequestDetail>(TERMINAL_BUFFER_ACTION_REQUEST_EVENT, {
      detail: { ...request, respond },
    }));
  });
}

export async function executeTerminalBufferAction(
  request: Omit<TerminalBufferActionRequestDetail, "respond">,
  target: Pick<EventTarget, "dispatchEvent"> = window,
): Promise<TerminalBufferActionPayload> {
  const payload = await requestTerminalBufferAction(request, target);
  if (payload.sessionId !== request.sessionId || payload.viewId !== request.viewId || payload.action !== request.action) {
    throw new Error("终端缓冲操作响应与目标视图不匹配。");
  }
  return payload;
}
