export const TERMINAL_FREE_INPUT_REQUEST_EVENT = "portmate-terminal-free-input";
export const MAX_TERMINAL_FREE_INPUT_CHARACTERS = 32_768;

export type TerminalFreeInputCut = {
  value: string;
  cutText: string;
  caret: number;
};

export function normalizeTerminalFreeInput(value: string): string {
  let normalized = "";
  let count = 0;
  for (const character of value) {
    if (count >= MAX_TERMINAL_FREE_INPUT_CHARACTERS) break;
    normalized += character;
    count += 1;
  }
  return normalized;
}

export function createTerminalFreeInputPayload(value: string): string | null {
  const normalized = normalizeTerminalFreeInput(value);
  if (!normalized) return null;
  return `${normalized.replace(/\r\n|\r|\n/g, "\r")}\r`;
}

export function terminalFreeInputCharacterCount(value: string): number {
  return Array.from(value).length;
}

export function cutTerminalFreeInputRange(value: string, start: number, end: number): TerminalFreeInputCut {
  const from = Math.max(0, Math.min(value.length, Math.trunc(start)));
  const to = Math.max(from, Math.min(value.length, Math.trunc(end)));
  return {
    value: `${value.slice(0, from)}${value.slice(to)}`,
    cutText: value.slice(from, to),
    caret: from,
  };
}

export function requestTerminalFreeInput(target: Pick<EventTarget, "dispatchEvent"> = window): boolean {
  return target.dispatchEvent(new Event(TERMINAL_FREE_INPUT_REQUEST_EVENT));
}
