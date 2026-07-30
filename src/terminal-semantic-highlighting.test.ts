import { describe, expect, it } from "vitest";
import {
  MAX_TERMINAL_SEMANTIC_LINE_CHARACTERS,
  terminalSemanticHighlightingEnabled,
  terminalSemanticHighlightingSupported,
  terminalSemanticTokens,
} from "./terminal-semantic-highlighting";

function classified(line: string) {
  const characters = Array.from(line);
  return terminalSemanticTokens(line).map((token) => ({
    kind: token.kind,
    text: characters.slice(token.start, token.end).join(""),
  }));
}

describe("terminal semantic highlighting", () => {
  it("classifies a Linux prompt command without coloring ordinary arguments", () => {
    expect(classified('root@OpenWrt:~# grep -n "wireless lan" /etc/config/wireless 192.168.1.1 42')).toEqual([
      { kind: "command", text: "grep" },
      { kind: "option", text: "-n" },
      { kind: "string", text: '"wireless lan"' },
      { kind: "path", text: "/etc/config/wireless" },
      { kind: "address", text: "192.168.1.1" },
      { kind: "number", text: "42" },
    ]);
  });

  it("recognizes commands after sudo and shell separators", () => {
    expect(classified("router# sudo -u root systemctl restart network && echo $STATUS")).toEqual([
      { kind: "command", text: "sudo" },
      { kind: "option", text: "-u" },
      { kind: "command", text: "systemctl" },
      { kind: "operator", text: "&&" },
      { kind: "command", text: "echo" },
      { kind: "variable", text: "$STATUS" },
    ]);
    expect(classified("router# env -u DEBUG MODE=prod systemctl status network")).toEqual([
      { kind: "command", text: "env" },
      { kind: "option", text: "-u" },
      { kind: "variable", text: "MODE=prod" },
      { kind: "command", text: "systemctl" },
    ]);
  });

  it("supports network-device and PowerShell prompts", () => {
    expect(classified("switch01> show interface ethernet 1")).toEqual([
      { kind: "command", text: "show" },
      { kind: "number", text: "1" },
    ]);
    expect(classified("PS C:\\Users\\ops> Get-Item C:\\Temp\\log.txt")).toEqual([
      { kind: "command", text: "Get-Item" },
      { kind: "path", text: "C:\\Temp\\log.txt" },
    ]);
  });

  it("does not color non-prompt output or percentages", () => {
    expect(terminalSemanticTokens("transfer progress 100% complete")).toEqual([]);
    expect(terminalSemanticTokens("normal program output")).toEqual([]);
  });

  it("fails closed for oversized or control-bearing prompt prefixes", () => {
    expect(terminalSemanticTokens(`${"p".repeat(MAX_TERMINAL_SEMANTIC_LINE_CHARACTERS + 1)}# ls`)).toEqual([]);
    expect(terminalSemanticTokens("root\u0000# ls")).toEqual([]);
  });

  it("defaults old preferences on and preserves an explicit opt-out", () => {
    expect(terminalSemanticHighlightingEnabled(null)).toBe(true);
    expect(terminalSemanticHighlightingEnabled({})).toBe(true);
    expect(terminalSemanticHighlightingEnabled({ semanticHighlightingEnabled: false })).toBe(false);
  });

  it("supports every interactive terminal transport, including serial", () => {
    expect(["serial", "shell", "ssh", "tcp", "telnet", "tmux"]
      .every(terminalSemanticHighlightingSupported)).toBe(true);
    expect(terminalSemanticHighlightingSupported("sftp")).toBe(false);
  });
});
