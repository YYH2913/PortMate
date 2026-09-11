import { normalizeCommandHistoryCommand } from "./command-history-state";

// Source describes the user action, not the transport's batching mode.
export type CommandSubmissionSource = "interactive" | "paste" | "free-input"
  | "quick-command" | "send-panel" | "mcp-run-command" | "mcp-send-text" | "sync-broadcast";

export type CommandSubmitHandler = (sessionId: string, command: string, source: CommandSubmissionSource) => void;

/** Extract only complete, history-safe commands from transport text. */
export function extractSubmittedCommands(
  text: string,
  source: CommandSubmissionSource,
): string[] {
  if (!text) return [];
  if (source === "mcp-send-text" || source === "send-panel") {
    if (Array.from(text).some((char) => /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/.test(char))) return [];
    const lines = text.split(/[\r\n]/u);
    return lines
      .filter((line, index) => source === "send-panel" || index < lines.length - 1)
      .map((line) => line.replace(/[\r\n]+$/u, ""))
      .filter((line) => Boolean(normalizeCommandHistoryCommand(line)?.trim()))
      .map((line) => normalizeCommandHistoryCommand(line)!)
      .filter((line) => !/[\u0000-\u001f\u007f-\u009f]/.test(line));
  }
  const command = text.replace(/[\r\n]+$/u, "");
  return normalizeCommandHistoryCommand(command)?.trim() && !/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/.test(command)
    ? [normalizeCommandHistoryCommand(command)!]
    : [];
}

export function recordCommandSubmission(
  record: CommandSubmitHandler,
  sessionId: string,
  command: string,
  source: CommandSubmissionSource,
): void {
  for (const submitted of extractSubmittedCommands(command, source)) {
    record(sessionId, submitted, source);
  }
}
