import { describe, expect, it } from "vitest";
import {
  SHELL_SESSION_IMPORT_MAX_SOURCE_CHARS,
  parseShellSessions,
} from "./shell-session-import";

describe("local Shell session import", () => {
  it("maps an /etc/shells-style list into independent local sessions", () => {
    const result = parseShellSessions(`# valid login shells
/bin/sh
/usr/bin/zsh
/bin/bash # preferred interactive shell
`);

    expect(result.error).toBeNull();
    expect(result.warnings).toEqual([]);
    expect(result.candidates).toEqual([
      { id: "shell-1-/bin/sh", name: "sh", program: "/bin/sh", args: [], warnings: [] },
      { id: "shell-2-/usr/bin/zsh", name: "zsh", program: "/usr/bin/zsh", args: [], warnings: [] },
      { id: "shell-3-/bin/bash", name: "bash", program: "/bin/bash", args: [], warnings: [] },
    ]);
  });

  it("supports Windows absolute paths and known system shell commands", () => {
    const result = parseShellSessions(`
"C:\\Program Files\\PowerShell\\7\\pwsh.exe"
cmd.exe
pwsh
`);

    expect(result.candidates).toEqual([
      expect.objectContaining({ name: "pwsh", program: "C:\\Program Files\\PowerShell\\7\\pwsh.exe", warnings: [] }),
      expect.objectContaining({ name: "cmd", program: "cmd.exe", warnings: [] }),
      expect.objectContaining({ name: "pwsh", program: "pwsh", warnings: [] }),
    ]);
  });

  it("preserves significant whitespace inside quoted absolute paths", () => {
    const result = parseShellSessions(`"/opt/ custom shell "`);

    expect(result.error).toBeNull();
    expect(result.warnings).toEqual([]);
    expect(result.candidates).toEqual([
      expect.objectContaining({ program: "/opt/ custom shell " }),
    ]);
  });

  it("rejects traversal, noninteractive programs, arbitrary commands, and duplicates", () => {
    const result = parseShellSessions(`
/bin/zsh
/bin/zsh
/usr/sbin/nologin
/bin/../bin/bash
/bin/bash -l
$(whoami)
`);

    expect(result.candidates).toEqual([
      expect.objectContaining({ name: "zsh", program: "/bin/zsh" }),
    ]);
    expect(result.warnings).toEqual(expect.arrayContaining([
      expect.stringContaining("/bin/zsh 重复"),
      expect.stringContaining("nologin 不是交互式 Shell"),
      expect.stringContaining("不是可直接导入的 Shell 路径"),
    ]));
  });

  it("rejects filesystem roots and directory-shaped paths", () => {
    const result = parseShellSessions(`
/
/bin/
C:\\
\\\\server\\share\\
`);

    expect(result.candidates).toEqual([]);
    expect(result.warnings.filter((warning) => warning.includes("不是可直接导入的 Shell 路径"))).toHaveLength(4);
  });

  it("bounds input before splitting it into lines", () => {
    const result = parseShellSessions("x".repeat(SHELL_SESSION_IMPORT_MAX_SOURCE_CHARS + 1));

    expect(result.candidates).toEqual([]);
    expect(result.warnings).toEqual([]);
    expect(result.error).toContain("字符限制");
  });
});
