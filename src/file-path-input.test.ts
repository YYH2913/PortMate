import { describe, expect, it } from "vitest";
import { exactNonBlankPathInput, parseFilePermissionMode } from "./file-path-input";

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
