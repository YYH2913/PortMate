import { describe, expect, it } from "vitest";
import { exactNonBlankPathInput, fileJoinPath, fileParentPath, parseFilePermissionMode } from "./file-path-input";

describe("exactNonBlankPathInput", () => {
  it("preserves significant leading and trailing whitespace", () => {
    expect(exactNonBlankPathInput(" /tmp/report.txt ")).toBe(" /tmp/report.txt ");
    expect(exactNonBlankPathInput("remote:/root/report.txt ")).toBe("remote:/root/report.txt ");
  });

  it("rejects cancelled, empty, and whitespace-only prompts", () => {
    expect(exactNonBlankPathInput(null)).toBeNull();
    expect(exactNonBlankPathInput("")).toBeNull();
    expect(exactNonBlankPathInput(" \t\r\n")).toBeNull();
  });
});

describe("file parent path", () => {
  it.each([
    ["F:\\folder", "F:\\"],
    ["F:\\folder\\", "F:\\"],
    ["F:\\", "F:\\"],
    ["F:/folder", "F:/"],
    ["F:/", "F:/"],
    ["F:\\folder\\nested", "F:\\folder"],
    ["F:\\folder/nested", "F:\\folder"],
    ["\\\\?\\F:\\folder", "\\\\?\\F:\\"],
    ["\\\\?\\F:\\", "\\\\?\\F:\\"],
    ["\\\\server\\share\\folder", "\\\\server\\share\\"],
    ["\\\\server\\share\\", "\\\\server\\share\\"],
    ["\\\\server\\share", "\\\\server\\share"],
    ["\\\\server\\share\\folder\\nested", "\\\\server\\share\\folder"],
    ["\\\\?\\UNC\\server\\share\\folder", "\\\\?\\UNC\\server\\share\\"],
    ["\\\\?\\UNC\\server\\share\\", "\\\\?\\UNC\\server\\share\\"],
    ["\\\\?\\UNC\\server\\share", "\\\\?\\UNC\\server\\share"],
    ["//server/share/folder", "//server/share/"],
    ["//server/share", "//server/share"],
  ])("preserves the Windows root when navigating up from %s", (path, expected) => {
    expect(fileParentPath(path, false)).toBe(expected);
  });

  it.each([
    ["/tmp/ report.txt ", "/tmp"],
    ["/ parent directory / report.txt ", "/ parent directory "],
    ["F:\\ parent directory \\ report.txt ", "F:\\ parent directory "],
    ["\\\\server\\ shared directory \\ report.txt ", "\\\\server\\ shared directory \\"],
    ["folder/file.txt", "folder"],
    ["folder\\file.txt", "folder"],
    ["file.txt", "."],
    [" file.txt ", "."],
    [".", ".."],
    ["", "."],
    ["/", "/"],
    ["~/folder", "~"],
    ["~\\folder", "~"],
    ["~/", "~"],
    ["~\\", "~"],
    ["~", "~"],
  ])("preserves local path whitespace and relative/home semantics for %s", (path, expected) => {
    expect(fileParentPath(path, false)).toBe(expected);
  });

  it.each([
    ["/tmp/name\\with\\slashes.txt", "/tmp"],
    ["/tmp\\directory/report.txt", "/tmp\\directory"],
    ["/tmp\\directory/report.txt\\", "/tmp\\directory"],
    ["name\\with\\slashes.txt", "."],
    ["file.txt", "."],
    ["/ parent / child ", "/ parent "],
    ["/folder/", "/"],
    ["/", "/"],
    ["", "."],
  ])("treats only forward slashes as remote POSIX separators in %s", (path, expected) => {
    expect(fileParentPath(path, true)).toBe(expected);
  });
});

describe("file path joining", () => {
  it.each([
    ["F:\\", "name.txt", "F:\\name.txt"],
    ["F:/", "name.txt", "F:/name.txt"],
    ["F:\\folder\\", "name.txt", "F:\\folder\\name.txt"],
    ["F:", "name.txt", "F:name.txt"],
    ["\\\\?\\F:\\", "name.txt", "\\\\?\\F:\\name.txt"],
    ["\\\\server\\share", "name.txt", "\\\\server\\share\\name.txt"],
    ["\\\\server\\share\\", "name.txt", "\\\\server\\share\\name.txt"],
    ["\\\\?\\UNC\\server\\share\\", "name.txt", "\\\\?\\UNC\\server\\share\\name.txt"],
    ["//server/share/", "name.txt", "//server/share/name.txt"],
    ["/", "name.txt", "/name.txt"],
    ["/ parent ", " report.txt ", "/ parent / report.txt "],
    ["F:\\ parent ", " report.txt ", "F:\\ parent \\ report.txt "],
    ["", " file.txt ", " file.txt "],
    [".", "file.txt", "./file.txt"],
    ["~", "file.txt", "~/file.txt"],
    ["~/", "file.txt", "~/file.txt"],
    ["~\\", "file.txt", "~\\file.txt"],
  ])("joins local %s and %s without losing the root or whitespace", (base, name, expected) => {
    expect(fileJoinPath(base, name, false)).toBe(expected);
  });

  it.each([
    ["/", "file.txt", "/file.txt"],
    ["/tmp/", "name\\part.txt", "/tmp/name\\part.txt"],
    ["/tmp\\", "file.txt", "/tmp\\/file.txt"],
    ["/ parent ", " report.txt ", "/ parent / report.txt "],
    ["folder\\", "file.txt", "folder\\/file.txt"],
    [".", "file.txt", "./file.txt"],
  ])("does not strip or reinterpret remote backslashes when joining %s", (base, name, expected) => {
    expect(fileJoinPath(base, name, true)).toBe(expected);
  });

  it("renames root children in place without switching drive or crossing a UNC share", () => {
    for (const directory of ["F:\\", "F:/", "\\\\?\\F:\\", "\\\\server\\share\\", "\\\\?\\UNC\\server\\share\\"]) {
      const path = `${directory}old name.txt`;
      expect(fileJoinPath(fileParentPath(path, false), "new name.txt", false)).toBe(`${directory}new name.txt`);
    }
    expect(fileJoinPath(fileParentPath("file.txt", true), "new.txt", true)).toBe("./new.txt");
  });
});

describe("relative directory parent navigation", () => {
  it.each([
    [".", ".."], ["./", ".."], ["..", "../.."], ["../", "../.."],
    ["../..", "../../.."], ["link/.", "link/./.."], ["link/..", "link/../.."],
    ["/home/link/..", "/home/link/../.."], ["dir\\..", "."],
  ])("navigates %s to %s without resolving remote symlinks in the UI", (path, expected) => {
    expect(fileParentPath(path, true)).toBe(expected);
  });
  it("preserves local Windows dot-component resolution", () => {
    expect(fileParentPath("F:\\link\\..", false)).toBe("F:\\link\\..\\..");
    expect(fileParentPath("~", false)).toBe("~");
  });
});

describe("permission mode input", () => {
  it("accepts full octal values including explicit prefixes", () => {
    expect(parseFilePermissionMode("0644")).toBe(0o644);
    expect(parseFilePermissionMode(" 0o755 ")).toBe(0o755);
    expect(parseFilePermissionMode("4755")).toBe(0o4755);
    expect(parseFilePermissionMode("000")).toBe(0);
  });
  it("rejects suffixes, partial octal digits, negative values and file type bits", () => {
    for (const value of ["644junk", "648", "-755", "77777", "100644", "7.55", "", "55"]) {
      expect(() => parseFilePermissionMode(value)).toThrow("八进制");
    }
  });
});
