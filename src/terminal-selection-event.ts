import type { TerminalKeyMode } from "./terminal-key-mode";

export const TERMINAL_SELECTION_REQUEST_EVENT = "portmate-terminal-selection";
export const TERMINAL_SELECTION_REQUEST_TIMEOUT_MS = 1_500;

export type TerminalSelectionAction = "copy" | "select-all" | "clear";

export type TerminalSelectionPayload = {
  sessionId: string;
  viewId: string;
  action: TerminalSelectionAction;
  selection: string | null;
};

export type TerminalSelectionResponse =
  | { ok: true; payload: TerminalSelectionPayload }
  | { ok: false; error: string };

export type TerminalSelectionRequestDetail = {
  sessionId: string;
  viewId: string;
  action: TerminalSelectionAction;
  respond: (response: TerminalSelectionResponse) => void;
};

type KeyboardShortcutLike = Pick<KeyboardEvent, "altKey" | "ctrlKey" | "key" | "metaKey" | "shiftKey">;

type MouseSelectionLike = Pick<MouseEvent,
  | "button"
  | "buttons"
  | "clientX"
  | "clientY"
  | "ctrlKey"
  | "metaKey"
  | "screenX"
  | "screenY"
>;

type ClipboardWriter = Pick<Clipboard, "writeText">;

export function terminalSelectionShortcut(
  event: KeyboardShortcutLike,
  mode: TerminalKeyMode,
): "copy" | "select-all" | null {
  if ((mode !== "remote" && mode !== "local")
    || !event.ctrlKey || !event.shiftKey || event.altKey || event.metaKey) return null;
  const key = event.key.toLowerCase();
  if (key === "c") return "copy";
  if (key === "a") return "select-all";
  return null;
}

export function terminalBlockSelectionMouseEventInit(
  event: MouseSelectionLike,
  forceSelection: boolean,
): MouseEventInit {
  return {
    altKey: true,
    bubbles: true,
    button: event.button,
    buttons: event.buttons,
    cancelable: true,
    clientX: event.clientX,
    clientY: event.clientY,
    composed: true,
    ctrlKey: event.ctrlKey,
    detail: 1,
    metaKey: event.metaKey,
    screenX: event.screenX,
    screenY: event.screenY,
    shiftKey: forceSelection,
  };
}

export function requestTerminalSelection(
  request: Omit<TerminalSelectionRequestDetail, "respond">,
  target: Pick<EventTarget, "dispatchEvent"> = window,
  timeoutMs = TERMINAL_SELECTION_REQUEST_TIMEOUT_MS,
): Promise<TerminalSelectionPayload> {
  return new Promise((resolve, reject) => {
    let settled = false;
    const timeout = setTimeout(() => {
      if (settled) return;
      settled = true;
      reject(new Error("未找到目标终端视图。"));
    }, timeoutMs);
    const respond = (response: TerminalSelectionResponse) => {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      if (response.ok) resolve(response.payload);
      else reject(new Error(response.error));
    };
    target.dispatchEvent(new CustomEvent<TerminalSelectionRequestDetail>(TERMINAL_SELECTION_REQUEST_EVENT, {
      detail: { ...request, respond },
    }));
  });
}

export async function executeTerminalSelectionAction(
  request: Omit<TerminalSelectionRequestDetail, "respond">,
  target: Pick<EventTarget, "dispatchEvent"> = window,
  clipboard: ClipboardWriter | undefined = navigator.clipboard,
): Promise<TerminalSelectionPayload> {
  const payload = await requestTerminalSelection(request, target);
  if (payload.sessionId !== request.sessionId || payload.viewId !== request.viewId || payload.action !== request.action) {
    throw new Error("终端选择响应与目标视图不匹配。");
  }
  if (request.action === "copy") {
    if (!payload.selection) throw new Error("当前终端没有选中文本。");
    if (!clipboard?.writeText) throw new Error("当前环境不支持写入剪贴板。");
    await clipboard.writeText(payload.selection);
  }
  return payload;
}
