import type { TerminalTextExportSource } from "./terminal-export-event";
import { terminalTextExportFileName } from "./terminal-export-path";

export function downloadTerminalText(
  text: string,
  sessionName: string,
  source: TerminalTextExportSource,
  createdAt = new Date(),
): string {
  const fileName = terminalTextExportFileName(sessionName, source, createdAt);
  const url = URL.createObjectURL(new Blob([text], { type: "text/plain;charset=utf-8" }));
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = fileName;
  anchor.style.display = "none";
  document.body.append(anchor);
  anchor.click();
  anchor.remove();
  setTimeout(() => URL.revokeObjectURL(url), 0);
  return fileName;
}
