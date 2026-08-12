import { describe, expect, it } from "vitest";
import {
  MAX_TERMINAL_EXPORT_DIRECTORY_CHARACTERS,
  normalizeTerminalExportDirectory,
  terminalTextExportFileName,
} from "./terminal-export-path";

describe("terminal export paths", () => {
  it("normalizes a bounded user-selected directory without changing platform separators", () => {
    expect(normalizeTerminalExportDirectory("  /home/operator/Exports  ")).toBe("/home/operator/Exports");
    expect(normalizeTerminalExportDirectory(" C:\\Users\\Operator\\Exports ")).toBe("C:\\Users\\Operator\\Exports");
    expect(normalizeTerminalExportDirectory("   ")).toBe("");
    expect(normalizeTerminalExportDirectory("bad\0path")).toBe("");
    expect(normalizeTerminalExportDirectory("x".repeat(MAX_TERMINAL_EXPORT_DIRECTORY_CHARACTERS + 1))).toBe("");
  });

  it("builds a portable, timestamped terminal text file name", () => {
    expect(terminalTextExportFileName(
      "Edge Router / 主机",
      "selection",
      new Date("2026-08-12T01:02:03.456Z"),
    )).toBe("Edge_Router-20260812T010203Z-selection.txt");
  });
});
