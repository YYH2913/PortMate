export const TERMINAL_GOTO_LINE_REQUEST_EVENT = "portmate-terminal-goto-line";

export function requestTerminalGotoLine(
  target: Pick<EventTarget, "dispatchEvent"> = window,
): boolean {
  return target.dispatchEvent(new Event(TERMINAL_GOTO_LINE_REQUEST_EVENT));
}
