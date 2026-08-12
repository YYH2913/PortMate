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

  it("recognizes commands after wrappers, assignments, and shell separators", () => {
    expect(classified("router# timeout --signal KILL 5s sudo -u root env MODE=prod /usr/bin/systemctl restart network && echo $STATUS")).toEqual([
      { kind: "command", text: "timeout" },
      { kind: "option", text: "--signal" },
      { kind: "number", text: "5s" },
      { kind: "command", text: "sudo" },
      { kind: "option", text: "-u" },
      { kind: "command", text: "env" },
      { kind: "variable", text: "MODE=prod" },
      { kind: "command", text: "/usr/bin/systemctl" },
      { kind: "operator", text: "&&" },
      { kind: "command", text: "echo" },
      { kind: "variable", text: "$STATUS" },
    ]);
  });

  it("supports tight CMD, PowerShell, network-device, U-Boot, fish, and glyph prompts", () => {
    expect(classified("C:\\Users\\ops>dir /b C:\\Temp\\log.txt")).toEqual([
      { kind: "command", text: "dir" },
      { kind: "option", text: "/b" },
      { kind: "path", text: "C:\\Temp\\log.txt" },
    ]);
    expect(classified("PS C:\\>Get-Item -Path C:\\Temp")).toEqual([
      { kind: "command", text: "Get-Item" },
      { kind: "option", text: "-Path" },
      { kind: "path", text: "C:\\Temp" },
    ]);
    expect(classified("switch(config-if)#show interface ethernet 1")).toEqual([
      { kind: "command", text: "show" },
      { kind: "number", text: "1" },
    ]);
    expect(classified("=> loady 0x80000000")).toEqual([
      { kind: "command", text: "loady" },
      { kind: "number", text: "0x80000000" },
    ]);
    expect(classified("user@router ~>ls /etc/config")).toEqual([
      { kind: "command", text: "ls" },
      { kind: "path", text: "/etc/config" },
    ]);
    expect(classified("~/PortMate on main ❯git status")).toEqual([
      { kind: "command", text: "git" },
    ]);
    expect(classified("λ npm test")).toEqual([
      { kind: "command", text: "npm" },
    ]);
  });

  it("highlights variables and nested commands inside double quotes", () => {
    expect(classified('host$ printf \'%s\\n\' "hello $USER $(git rev-parse --show-toplevel)" >"$OUT"')).toEqual([
      { kind: "command", text: "printf" },
      { kind: "string", text: "'%s\\n'" },
      { kind: "string", text: '"hello ' },
      { kind: "variable", text: "$USER" },
      { kind: "string", text: " " },
      { kind: "operator", text: "$(" },
      { kind: "command", text: "git" },
      { kind: "option", text: "--show-toplevel" },
      { kind: "operator", text: ")" },
      { kind: "string", text: '"' },
      { kind: "operator", text: ">" },
      { kind: "string", text: '"' },
      { kind: "variable", text: "$OUT" },
      { kind: "string", text: '"' },
    ]);
    expect(classified('host$ echo "say \\"hello\\" to $env:USERNAME"')).toEqual([
      { kind: "command", text: "echo" },
      { kind: "string", text: '"say \\"hello\\" to ' },
      { kind: "variable", text: "$env:USERNAME" },
      { kind: "string", text: '"' },
    ]);
  });

  it("recognizes CMD and PowerShell environment variables", () => {
    expect(classified("C:\\>%SystemRoot%\\System32\\where.exe !PATHEXT!")).toEqual([
      { kind: "variable", text: "%SystemRoot%" },
      { kind: "variable", text: "!PATHEXT!" },
    ]);
    expect(classified("PS C:\\>Write-Output $env:ProgramFiles")).toEqual([
      { kind: "command", text: "Write-Output" },
      { kind: "variable", text: "$env:ProgramFiles" },
    ]);
  });

  it("handles nested substitutions, multi-character redirects, and pipelines", () => {
    expect(classified('host$ cat <$(printf "%s" "$(uname -m)") 2>>error.log |& sed -n \'1p\'')).toEqual([
      { kind: "command", text: "cat" },
      { kind: "operator", text: "<" },
      { kind: "operator", text: "$(" },
      { kind: "command", text: "printf" },
      { kind: "string", text: '"%s"' },
      { kind: "string", text: '"' },
      { kind: "operator", text: "$(" },
      { kind: "command", text: "uname" },
      { kind: "option", text: "-m" },
      { kind: "operator", text: ")" },
      { kind: "string", text: '"' },
      { kind: "operator", text: ")" },
      { kind: "operator", text: "2>>" },
      { kind: "path", text: "error.log" },
      { kind: "operator", text: "|&" },
      { kind: "command", text: "sed" },
      { kind: "option", text: "-n" },
      { kind: "string", text: "'1p'" },
    ]);
  });

  it("treats shell arrays as arguments rather than commands", () => {
    expect(classified('host$ files=(/tmp/a "$HOME/b"); printf \'%s\\n\' "${files[@]}"')).toEqual([
      { kind: "variable", text: "files=" },
      { kind: "operator", text: "(" },
      { kind: "path", text: "/tmp/a" },
      { kind: "string", text: '"' },
      { kind: "variable", text: "$HOME" },
      { kind: "string", text: '/b"' },
      { kind: "operator", text: ")" },
      { kind: "operator", text: ";" },
      { kind: "command", text: "printf" },
      { kind: "string", text: "'%s\\n'" },
      { kind: "string", text: '"' },
      { kind: "variable", text: "${files[@]}" },
      { kind: "string", text: '"' },
    ]);
  });

  it("keeps token offsets in Unicode code points", () => {
    expect(classified('路由器(config)#echo "你好" /tmp/固件.bin')).toEqual([
      { kind: "command", text: "echo" },
      { kind: "string", text: '"你好"' },
      { kind: "path", text: "/tmp/固件.bin" },
    ]);
  });

  it("does not color non-prompt output, percentages, or spaced prose markers", () => {
    expect(terminalSemanticTokens("transfer progress 100% complete")).toEqual([]);
    expect(terminalSemanticTokens("100% complete")).toEqual([]);
    expect(terminalSemanticTokens("normal program output")).toEqual([]);
    expect(terminalSemanticTokens("Issue # status remains open")).toEqual([]);
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
