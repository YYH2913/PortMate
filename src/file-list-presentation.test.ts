import { describe, expect, it } from "vitest";
import {
  defaultFileSort, fileModifiedTimestamp, fileNameExtension, filePermissionDescription,
  knownFileSize, nextFileSort, sortFileEntries, summarizeFileEntries,
} from "./file-list-presentation";
import { updateFileSelection } from "./file-selection";
import type { FileEntry } from "./types";

function entry(name: string, overrides: Partial<FileEntry> = {}): FileEntry {
  return { name, path: `/data/${name}`, isDir: false, size: 0, ...overrides };
}
const names = (entries: FileEntry[]) => entries.map(item => item.name);

describe("file list presentation", () => {
  it("naturally sorts file numbers and keeps directories first in both directions", () => {
    const source = [entry("file10"), entry("folder2", { isDir: true }), entry("file2"), entry("folder1", { isDir: true })];
    expect(names(sortFileEntries(source, defaultFileSort))).toEqual(["folder1", "folder2", "file2", "file10"]);
    expect(names(sortFileEntries(source, { key: "name", direction: "desc" }))).toEqual(["folder2", "folder1", "file10", "file2"]);
    expect(names(source)).toEqual(["file10", "folder2", "file2", "folder1"]);
  });

  it("compares suffixes without treating simple dotfiles or trailing dots as extensions", () => {
    expect([".gitignore", ".env.local", "archive.tar.GZ", "README", "file."].map(fileNameExtension))
      .toEqual(["", "local", "gz", "", ""]);
    const source = [entry("b.ZIP"), entry("z.txt"), entry("c"), entry("a.TXT")];
    expect(names(sortFileEntries(source, { key: "type", direction: "asc" }))).toEqual(["c", "a.TXT", "z.txt", "b.ZIP"]);
    expect(names(sortFileEntries(source, { key: "type", direction: "desc" }))).toEqual(["b.ZIP", "a.TXT", "z.txt", "c"]);
  });

  it("sorts real file sizes, not the formatted text, and ignores directory inode sizes", () => {
    const source = [entry("large", { size: 2048 }), entry("folder2", { isDir: true, size: 1 }), entry("small", { size: 512 }), entry("folder1", { isDir: true, size: 9000 }), entry("empty"), entry("missing", { size: NaN })];
    expect(names(sortFileEntries(source, { key: "size", direction: "asc" }))).toEqual(["folder1", "folder2", "empty", "small", "large", "missing"]);
    expect(names(sortFileEntries(source, { key: "size", direction: "desc" }))).toEqual(["folder1", "folder2", "large", "small", "empty", "missing"]);
  });

  it("compares actual instants and puts absent/invalid timestamps last in either direction", () => {
    const source = [entry("absent"), entry("new", { modified: "2026-09-16T02:00:00Z" }), entry("invalid", { modified: "not-a-date" }), entry("old", { modified: "2026-09-16T09:00:00+08:00" })];
    expect(names(sortFileEntries(source, { key: "modified", direction: "asc" }))).toEqual(["old", "new", "absent", "invalid"]);
    expect(names(sortFileEntries(source, { key: "modified", direction: "desc" }))).toEqual(["new", "old", "absent", "invalid"]);
    expect(fileModifiedTimestamp("1970-01-01T00:00:00Z")).toBe(0);
    expect(fileModifiedTimestamp("bad")).toBeNull();
  });

  it("uses deterministic ties without changing names or path bytes", () => {
    const source = [entry("Report2", { path: "C:\\dir\\Report2 " }), entry("report02", { path: "C:\\dir\\report02" })];
    const snapshot = structuredClone(source);
    expect(sortFileEntries(source, defaultFileSort)).toEqual(sortFileEntries([...source].reverse(), defaultFileSort));
    expect(source).toEqual(snapshot);
  });

  it("toggles only the requested pane's sort, with English field and direction identifiers", () => {
    const local = nextFileSort(defaultFileSort, "size");
    expect(local).toEqual({ key: "size", direction: "asc" });
    expect(nextFileSort(local, "size")).toEqual({ key: "size", direction: "desc" });
    expect(nextFileSort(local, "modified")).toEqual({ key: "modified", direction: "asc" });
    expect(defaultFileSort).toEqual({ key: "name", direction: "asc" });
  });

  it("selects Shift ranges in displayed sort order rather than backend order", () => {
    const source = [entry("file10"), entry("file1"), entry("file3"), entry("file2")];
    const displayed = sortFileEntries(source, defaultFileSort);
    const result = updateFileSelection(displayed, [], source[2], source[1].path, { shiftKey: true, ctrlKey: false, metaKey: false });
    expect(names(result.selected)).toEqual(["file1", "file2", "file3"]);
  });

  it("summarizes direct file bytes, excluding directories and unusable metadata", () => {
    expect(summarizeFileEntries([entry("file", { size: 100 }), entry("empty"), entry("folder", { isDir: true, size: 4096 }), entry("unknown", { size: -1 })]))
      .toEqual({ count: 4, files: 3, directories: 1, bytes: 100, unknownSizes: 1 });
    expect(knownFileSize(Infinity)).toBeNull();
    expect(knownFileSize(0)).toBe(0);
  });

  it("shows octal and symbolic permissions including special bits, without masking missing values", () => {
    expect(filePermissionDescription(0o100644)).toBe("0644 (rw-r--r--)");
    expect(filePermissionDescription(0o104755)).toBe("04755 (rwsr-xr-x)");
    expect(filePermissionDescription(0o103640)).toBe("03640 (rw-r-S--T)");
    expect(filePermissionDescription(0)).toBe("0000 (---------)");
    expect(filePermissionDescription(null)).toBeNull();
    expect(filePermissionDescription(-1)).toBeNull();
  });
});
