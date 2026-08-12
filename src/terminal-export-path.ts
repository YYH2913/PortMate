import type { TerminalTextExportSource } from "./terminal-export-event";

export const MAX_TERMINAL_EXPORT_DIRECTORY_CHARACTERS = 32_768;

export function normalizeTerminalExportDirectory(value: unknown): string {
  if (typeof value !== "string") return "";
  const directory = value.trim();
  if (!directory || directory.length > MAX_TERMINAL_EXPORT_DIRECTORY_CHARACTERS || directory.includes("\0")) {
    return "";
  }
  return directory;
}

export function terminalTextExportFileName(
  sessionName: string,
  source: TerminalTextExportSource,
  createdAt = new Date(),
): string {
  const safeName = sessionName
    .replace(/[^A-Za-z0-9._-]+/g, "_")
    .replace(/^_+|_+$/g, "")
    .slice(0, 80) || "session";
  const timestamp = createdAt.toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
  return `${safeName}-${timestamp}-${source}.txt`;
}

export async function chooseTerminalExportDirectory(defaultPath: string): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const selected = await open({
    title: "选择终端文本导出目录",
    directory: true,
    multiple: false,
    canCreateDirectories: true,
    defaultPath: normalizeTerminalExportDirectory(defaultPath) || undefined,
  });
  return typeof selected === "string" ? selected : null;
}

export async function chooseTerminalTextExportPath(
  defaultDirectory: string,
  fileName: string,
): Promise<string | null> {
  const normalizedDirectory = normalizeTerminalExportDirectory(defaultDirectory);
  const defaultPath = normalizedDirectory
    ? await import("@tauri-apps/api/path").then(({ join }) => join(normalizedDirectory, fileName))
    : fileName;
  const { save } = await import("@tauri-apps/plugin-dialog");
  return save({
    title: "导出终端文本到",
    defaultPath,
    canCreateDirectories: true,
    filters: [{ name: "文本文件", extensions: ["txt"] }],
  });
}
