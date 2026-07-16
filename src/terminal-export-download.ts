import type { TerminalTextExportSource } from "./terminal-export-event";

export function downloadTerminalText(
  text: string,
  sessionName: string,
  source: TerminalTextExportSource,
  createdAt = new Date(),
): string {
  const safeName = sessionName.replace(/[^A-Za-z0-9._-]+/g, "_").replace(/^_+|_+$/g, "").slice(0, 80) || "session";
  const timestamp = createdAt.toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
  const fileName = `${safeName}-${timestamp}-${source}.txt`;
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
