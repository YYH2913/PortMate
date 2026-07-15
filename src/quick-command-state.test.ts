import { describe, expect, it } from "vitest";
import {
  createQuickCommand,
  limitQuickCommandLabelInput,
  MAX_QUICK_COMMAND_LABEL_CHARACTERS,
  MAX_QUICK_COMMAND_TEXT_CHARACTERS,
  MAX_QUICK_COMMANDS,
  moveQuickCommand,
  normalizeQuickCommandLibrary,
  quickCommandPayload,
} from "./quick-command-state";

describe("quick command state", () => {
  it("migrates legacy arrays and applies explicit text bounds", () => {
    const library = normalizeQuickCommandLibrary([
      { name: ` L${"😀".repeat(80)} `, text: `${"x".repeat(MAX_QUICK_COMMAND_TEXT_CHARACTERS)}tail`, id: "legacy", appendEnter: false },
      { name: "missing text" },
    ]);
    expect(library.version).toBe(1);
    expect(Array.from(library.items[0].label)).toHaveLength(MAX_QUICK_COMMAND_LABEL_CHARACTERS);
    expect(Array.from(library.items[0].command)).toHaveLength(MAX_QUICK_COMMAND_TEXT_CHARACTERS);
    expect(library.items[0]).toMatchObject({ id: "legacy", appendEnter: false });
  });

  it("repairs duplicate IDs and caps the stored collection", () => {
    const library = normalizeQuickCommandLibrary({
      version: 1,
      items: Array.from({ length: MAX_QUICK_COMMANDS + 4 }, (_, index) => ({
        id: "same",
        label: `Command ${index}`,
        command: `echo ${index}`,
      })),
    }, () => "same");
    expect(library.items).toHaveLength(MAX_QUICK_COMMANDS);
    expect(new Set(library.items.map((item) => item.id)).size).toBe(MAX_QUICK_COMMANDS);
  });

  it("creates only complete commands and preserves whitespace in command text", () => {
    expect(createQuickCommand({ label: "  Status  ", command: " git status ", appendEnter: true }, "quick:status")).toEqual({
      id: "quick:status",
      label: "Status",
      command: " git status ",
      appendEnter: true,
    });
    expect(createQuickCommand({ label: "", command: "pwd", appendEnter: true })).toBeNull();
  });

  it("limits label input without trimming spaces while editing", () => {
    expect(limitQuickCommandLabelInput("  deploy  ")).toBe("  deploy  ");
    expect(limitQuickCommandLabelInput(`A\u0000${"😀".repeat(80)}`)).not.toContain("\u0000");
    expect(Array.from(limitQuickCommandLabelInput(`A${"😀".repeat(80)}`))).toHaveLength(MAX_QUICK_COMMAND_LABEL_CHARACTERS);
  });

  it("builds insert and execute payloads without changing the command body", () => {
    const command = { id: "quick:1", label: "Status", command: "git status --short", appendEnter: false };
    expect(quickCommandPayload(command)).toBe("git status --short");
    expect(quickCommandPayload({ ...command, appendEnter: true })).toBe("git status --short\r");
  });

  it("moves commands within bounds without mutating the source", () => {
    const items = [
      { id: "a", label: "A", command: "a", appendEnter: true },
      { id: "b", label: "B", command: "b", appendEnter: true },
    ];
    expect(moveQuickCommand(items, "b", -1).map((item) => item.id)).toEqual(["b", "a"]);
    expect(moveQuickCommand(items, "a", -1)).toBe(items);
    expect(items.map((item) => item.id)).toEqual(["a", "b"]);
  });
});
