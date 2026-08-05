import { describe, expect, it } from "vitest";
import {
  emptyTerminalCompletionInputState,
  reduceTerminalCompletionInput,
  terminalCompletionSourceLabel,
  terminalCompletionSuggestions,
  terminalCompletionSupported,
  terminalCompletionUsageHint,
} from "./terminal-completion-state";
import {
  defaultTerminalCompletionPreferences,
  normalizeTerminalCompletionPreferences,
  terminalCompletionPreferencesFromSettings,
} from "./terminal-completion-prefs";

describe("terminal completion state", () => {
  it("normalizes damaged preferences to bounded defaults", () => {
    expect(normalizeTerminalCompletionPreferences({
      enabled: false,
      commandNames: "yes",
      triggerCharacters: 3,
      listRows: 10,
      previewMode: "input",
    })).toEqual({
      ...defaultTerminalCompletionPreferences,
      enabled: false,
      triggerCharacters: 3,
      listRows: 10,
      previewMode: "input",
    });
    expect(normalizeTerminalCompletionPreferences({ triggerCharacters: 9, listRows: 99, previewMode: "invalid" }))
      .toEqual(defaultTerminalCompletionPreferences);
    expect(terminalCompletionPreferencesFromSettings({
      completionEnabled: true,
      completionCommandNames: false,
      completionTriggerChars: "3 字符",
      completionListHeight: "10 行",
      completionPreviewMode: "列表顶部",
    })).toEqual({
      ...defaultTerminalCompletionPreferences,
      commandNames: false,
      triggerCharacters: 3,
      listRows: 10,
      previewMode: "top",
    });
  });

  it("tracks only append-only terminal lines and resynchronizes at boundaries", () => {
    let state = reduceTerminalCompletionInput(emptyTerminalCompletionInputState, "git sta");
    expect(state).toEqual({ line: "git sta", synchronized: true });
    state = reduceTerminalCompletionInput(state, "\u007fatus");
    expect(state.line).toBe("git status");
    state = reduceTerminalCompletionInput(state, "\u0017diff");
    expect(state.line).toBe("git diff");
    state = reduceTerminalCompletionInput(state, "\u0015");
    expect(state.line).toBe("");
    state = reduceTerminalCompletionInput(state, "git status");
    state = reduceTerminalCompletionInput(state, "\u001b[A");
    expect(state).toEqual({ line: "", synchronized: false });
    expect(reduceTerminalCompletionInput(state, "ignored").synchronized).toBe(false);
    expect(reduceTerminalCompletionInput(state, "\rnext")).toEqual({ line: "next", synchronized: true });
  });

  it("suggests commands, options, and subcommands as append-only suffixes", () => {
    const command = terminalCompletionSuggestions({
      line: "gi",
      preferences: defaultTerminalCompletionPreferences,
    });
    expect(command.find((item) => item.label === "git")).toMatchObject({
      source: "command",
      appendText: "t ",
    });

    const argument = terminalCompletionSuggestions({
      line: "git st",
      preferences: defaultTerminalCompletionPreferences,
    });
    expect(argument.find((item) => item.label === "status")).toMatchObject({
      source: "subcommand",
      appendText: "atus ",
    });

    const option = terminalCompletionSuggestions({
      line: "ssh -p",
      preferences: defaultTerminalCompletionPreferences,
    });
    expect(option.find((item) => item.label === "-p")?.appendText).toBe(" ");
  });

  it("uses nested schemas for subcommand options and real positional values", () => {
    const commit = terminalCompletionSuggestions({
      line: "git commit --a",
      preferences: defaultTerminalCompletionPreferences,
    });
    expect(commit.find((item) => item.label === "--amend")).toMatchObject({
      source: "option",
      appendText: "mend ",
    });

    const compose = terminalCompletionSuggestions({
      line: "docker compose u",
      preferences: defaultTerminalCompletionPreferences,
    });
    expect(compose.find((item) => item.label === "up")).toMatchObject({
      source: "subcommand",
      appendText: "p ",
    });

    const mode = terminalCompletionSuggestions({
      line: "chmod 7",
      preferences: defaultTerminalCompletionPreferences,
    });
    expect(mode.some((item) => item.source === "argument" && item.label === "755")).toBe(true);
  });

  it("provides non-inserting usage hints for every known command context", () => {
    expect(terminalCompletionUsageHint({
      line: "ls ",
      preferences: defaultTerminalCompletionPreferences,
    })).toEqual({ label: "ls [选项] [路径...]", detail: "列出目录内容" });
    expect(terminalCompletionUsageHint({
      line: "git commit -m",
      preferences: defaultTerminalCompletionPreferences,
    })).toEqual({ label: "git commit [选项] [路径...]", detail: "提交暂存变更" });
    expect(terminalCompletionUsageHint({
      line: "docker compose up ",
      preferences: defaultTerminalCompletionPreferences,
    })).toEqual({ label: "docker compose up [选项] [服务...]", detail: "创建并启动服务" });
    expect(terminalCompletionUsageHint({
      line: "unknown-command ",
      preferences: defaultTerminalCompletionPreferences,
    })).toBeNull();
    expect(terminalCompletionUsageHint({
      line: "git status | grep M",
      preferences: defaultTerminalCompletionPreferences,
    })).toBeNull();
  });

  it("supports all interactive transports and names subcommands accurately", () => {
    expect(["serial", "shell", "ssh", "tcp", "telnet", "tmux"].every(terminalCompletionSupported))
      .toBe(true);
    expect(terminalCompletionSupported("sftp")).toBe(false);
    expect(terminalCompletionSourceLabel("subcommand")).toBe("子命令");
  });

  it("ranks exact Quick Commands and explicit history without accepting multiline payloads", () => {
    const suggestions = terminalCompletionSuggestions({
      line: "git s",
      preferences: defaultTerminalCompletionPreferences,
      history: ["git status", "git\nsecret"],
      quickCommands: [
        { id: "status", label: "仓库状态", command: "git status" },
        { id: "multi", label: "多行", command: "git status\ngit diff" },
        { id: "operator", label: "复合命令", command: "git status; rm output" },
      ],
    });
    expect(suggestions[0]).toMatchObject({ source: "quick", label: "仓库状态", appendText: "tatus" });
    expect(suggestions.some((item) => item.target.includes("\n"))).toBe(false);
    expect(suggestions.some((item) => item.target.includes(";"))).toBe(false);
    expect(suggestions.filter((item) => item.target === "git status")).toHaveLength(1);
  });

  it("honors source switches, trigger length, and conservative shell syntax boundaries", () => {
    const preferences = {
      ...defaultTerminalCompletionPreferences,
      commandNames: false,
      commandArgs: false,
      commandOptions: false,
      history: false,
      quickCommands: true,
      triggerCharacters: 3 as const,
    };
    expect(terminalCompletionSuggestions({
      line: "gi",
      preferences,
      quickCommands: [{ id: "one", label: "状态", command: "git status" }],
    })).toEqual([]);
    expect(terminalCompletionSuggestions({
      line: "git",
      preferences,
      quickCommands: [{ id: "one", label: "状态", command: "git status" }],
    })[0]?.source).toBe("quick");
    expect(terminalCompletionSuggestions({
      line: "git status | gr",
      preferences: defaultTerminalCompletionPreferences,
    })).toEqual([]);
  });
});
