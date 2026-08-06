import { describe, expect, it } from "vitest";
import { terminalCommandCatalog } from "./terminal-command-catalog";
import type { TerminalCommandSchema } from "./terminal-command-catalog";

describe("terminal command catalog", () => {
  it("keeps command, option, argument, and nested subcommand keys unique", () => {
    expect(new Set(terminalCommandCatalog.map((command) => command.value)).size)
      .toBe(terminalCommandCatalog.length);
    for (const command of terminalCommandCatalog) assertSchema(command);
  });

  it("provides usage metadata for every built-in command", () => {
    const commands = terminalCommandCatalog.map((command) => command.value);
    expect(commands).toHaveLength(128);
    expect(commands).toEqual(expect.arrayContaining([
      "ansible-playbook",
      "apk",
      "apt",
      "cmd",
      "dnf",
      "docker",
      "dotnet",
      "git",
      "go",
      "helm",
      "ip",
      "kubectl",
      "ls",
      "pip3",
      "podman",
      "python3",
      "sftp",
      "ssh",
      "terraform",
      "tmux",
      "winget",
      "yum",
    ]));
    expect(terminalCommandCatalog.every((command) => command.usage.startsWith(command.value))).toBe(true);
  });
});

function assertSchema(schema: TerminalCommandSchema) {
  expect(schema.value.trim()).toBe(schema.value);
  expect(schema.value).not.toBe("");
  expect(schema.detail).not.toBe("");
  expect(schema.usage).not.toBe("");
  expect(new Set(schema.options.map((entry) => entry.value)).size).toBe(schema.options.length);
  expect(schema.options.filter((entry) => entry.takesValue).every((entry) => entry.value.startsWith("-") || entry.value.startsWith("/"))).toBe(true);
  expect(new Set(schema.arguments.map((entry) => entry.value)).size).toBe(schema.arguments.length);
  expect(new Set(schema.subcommands.map((entry) => entry.value)).size).toBe(schema.subcommands.length);
  for (const subcommand of schema.subcommands) assertSchema(subcommand);
}
