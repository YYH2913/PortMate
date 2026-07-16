export const TERMINAL_GOTO_LINE_REQUEST_EVENT = "portmate-terminal-goto-line";
export const MAX_TERMINAL_GOTO_LINE_QUERY_LENGTH = 16;

export type TerminalGotoLineResolution =
  | { kind: "empty" }
  | { kind: "invalid" }
  | { kind: "out-of-range" }
  | { kind: "valid"; targetLine: number; relative: boolean };

export function resolveTerminalGotoLine(
  query: string,
  currentLine: number,
  lineCount: number,
): TerminalGotoLineResolution {
  const normalized = query.trim();
  if (!normalized) return { kind: "empty" };
  const relative = /^[+-]\d+$/.test(normalized);
  if (!relative && !/^\d+$/.test(normalized)) return { kind: "invalid" };
  const parsed = Number(normalized);
  if (!Number.isSafeInteger(parsed)) return { kind: "invalid" };
  const targetLine = relative ? currentLine + parsed : parsed;
  if (targetLine < 1 || targetLine > lineCount) return { kind: "out-of-range" };
  return { kind: "valid", targetLine, relative };
}

export function terminalGotoViewportLine(
  targetLine: number,
  rows: number,
  lineCount: number,
): number {
  const safeRows = Math.max(1, Math.trunc(rows));
  const safeLineCount = Math.max(1, Math.trunc(lineCount));
  const targetIndex = Math.min(safeLineCount - 1, Math.max(0, Math.trunc(targetLine) - 1));
  const maxViewport = Math.max(0, safeLineCount - safeRows);
  return Math.min(maxViewport, Math.max(0, targetIndex - Math.floor(safeRows / 2)));
}

export function terminalGotoCurrentLine(
  viewportLine: number,
  rows: number,
  lineCount: number,
): number {
  const safeLineCount = Math.max(1, Math.trunc(lineCount));
  const viewportCenter = Math.max(0, Math.trunc(viewportLine)) + Math.floor(Math.max(1, Math.trunc(rows)) / 2);
  return Math.min(safeLineCount, viewportCenter + 1);
}

export function terminalGotoLineStatus(
  resolution: TerminalGotoLineResolution,
  currentLine: number,
  lineCount: number,
): string {
  if (resolution.kind === "empty") return `当前 ${currentLine} / 共 ${lineCount}`;
  if (resolution.kind === "invalid") return "请输入行号";
  if (resolution.kind === "out-of-range") return `范围 1..${lineCount}`;
  return `目标 ${resolution.targetLine} / 共 ${lineCount}`;
}

export function requestTerminalGotoLine(
  target: Pick<EventTarget, "dispatchEvent"> = window,
): boolean {
  return target.dispatchEvent(new Event(TERMINAL_GOTO_LINE_REQUEST_EVENT));
}
