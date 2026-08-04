import { describe, expect, it } from "vitest";
import { exactNonBlankPathInput } from "./file-path-input";

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
