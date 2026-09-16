import type { FileEntry } from "./types";

export const fileSortKeys = ["name", "type", "size", "modified"] as const;
export type FileSortKey = typeof fileSortKeys[number];
export type FileSort = Readonly<{ key: FileSortKey; direction: "asc" | "desc" }>;
export const defaultFileSort: FileSort = { key: "name", direction: "asc" };

// Keep ordering stable when only the UI language changes. Names and paths are never rewritten.
const naturalOrder = new Intl.Collator("en", { numeric: true, sensitivity: "base" });

export function nextFileSort(current: FileSort, key: FileSortKey): FileSort {
  return { key, direction: current.key === key && current.direction === "asc" ? "desc" : "asc" };
}

/** Dotfiles without a suffix and trailing dots are not extensions. */
export function fileNameExtension(name: string): string {
  const index = name.lastIndexOf(".");
  return index > 0 && index < name.length - 1 ? name.slice(index + 1).toLowerCase() : "";
}

export function fileModifiedTimestamp(value?: string | null): number | null {
  if (!value) return null;
  const timestamp = Date.parse(value);
  return Number.isFinite(timestamp) ? timestamp : null;
}

export function knownFileSize(value: number): number | null {
  return Number.isFinite(value) && value >= 0 ? value : null;
}

function compareKnownNumbers(a: number | null, b: number | null, direction: number): number {
  // Missing metadata remains at the end in both directions, not ahead of real values.
  if (a === null || b === null) return a === b ? 0 : a === null ? 1 : -1;
  return (a - b) * direction;
}

export function sortFileEntries(entries: readonly FileEntry[], sort: FileSort): FileEntry[] {
  const direction = sort.direction === "asc" ? 1 : -1;
  return [...entries].sort((a, b) => {
    // Directory inode sizes are not recursive content sizes; don't sort by those values.
    if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
    let result = 0;
    if (sort.key === "type") {
      result = naturalOrder.compare(a.isDir ? "" : fileNameExtension(a.name), b.isDir ? "" : fileNameExtension(b.name)) * direction;
    } else if (sort.key === "size" && !a.isDir) {
      result = compareKnownNumbers(knownFileSize(a.size), knownFileSize(b.size), direction);
    } else if (sort.key === "modified") {
      result = compareKnownNumbers(fileModifiedTimestamp(a.modified), fileModifiedTimestamp(b.modified), direction);
    }
    if (result) return result;
    const nameOrder = naturalOrder.compare(a.name, b.name);
    if (nameOrder) return nameOrder * (sort.key === "name" ? direction : 1);
    // An exact tie breaker makes case variants and numerically equivalent names deterministic.
    return a.path < b.path ? -1 : a.path > b.path ? 1 : 0;
  });
}

export function summarizeFileEntries(entries: readonly FileEntry[]) {
  let files = 0;
  let directories = 0;
  let bytes = 0;
  let unknownSizes = 0;
  for (const entry of entries) {
    if (entry.isDir) directories += 1;
    else {
      files += 1;
      const size = knownFileSize(entry.size);
      if (size === null) unknownSizes += 1;
      else bytes += size;
    }
  }
  return { count: entries.length, files, directories, bytes, unknownSizes };
}

/** Permission bits only; file type is presented separately. */
export function filePermissionDescription(mode?: number | null): string | null {
  if (mode == null || !Number.isInteger(mode) || mode < 0) return null;
  const symbols = [0o400, 0o200, 0o100, 0o040, 0o020, 0o010, 0o004, 0o002, 0o001]
    .map((bit, index) => mode & bit ? "rwx"[index % 3] : "-");
  if (mode & 0o4000) symbols[2] = mode & 0o100 ? "s" : "S";
  if (mode & 0o2000) symbols[5] = mode & 0o010 ? "s" : "S";
  if (mode & 0o1000) symbols[8] = mode & 0o001 ? "t" : "T";
  return `0${(mode & 0o7777).toString(8).padStart(3, "0")} (${symbols.join("")})`;
}
