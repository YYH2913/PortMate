import type { TerminalCompletionPreferences, TerminalCompletionQuickCommand } from "./terminal-completion-prefs";
export type { TerminalCompletionPreferences, TerminalCompletionQuickCommand } from "./terminal-completion-prefs";

export type TerminalCompletionSource = "command" | "option" | "argument" | "history" | "quick";

export type TerminalCompletionSuggestion = {
  id: string;
  source: TerminalCompletionSource;
  label: string;
  detail: string;
  target: string;
  appendText: string;
};

export type TerminalCompletionInputState = {
  line: string;
  synchronized: boolean;
};

export const emptyTerminalCompletionInputState: TerminalCompletionInputState = {
  line: "",
  synchronized: true,
};

const MAX_COMPLETION_LINE_CHARACTERS = 512;
const MAX_COMPLETION_CANDIDATES = 40;

type CatalogEntry = { value: string; detail: string };

const commandCatalog: CatalogEntry[] = [
  { value: "bash", detail: "启动 Bash shell" },
  { value: "cargo", detail: "Rust 构建与包管理" },
  { value: "cat", detail: "输出文件内容" },
  { value: "cd", detail: "切换当前目录" },
  { value: "chmod", detail: "修改文件权限" },
  { value: "chown", detail: "修改文件所有者" },
  { value: "clear", detail: "清空终端屏幕" },
  { value: "cp", detail: "复制文件或目录" },
  { value: "curl", detail: "传输 URL 数据" },
  { value: "docker", detail: "管理容器与镜像" },
  { value: "echo", detail: "输出文本" },
  { value: "find", detail: "查找文件" },
  { value: "git", detail: "管理 Git 仓库" },
  { value: "grep", detail: "搜索文本" },
  { value: "head", detail: "输出文件开头" },
  { value: "journalctl", detail: "查询 systemd 日志" },
  { value: "kill", detail: "向进程发送信号" },
  { value: "ls", detail: "列出目录内容" },
  { value: "mkdir", detail: "创建目录" },
  { value: "mv", detail: "移动或重命名文件" },
  { value: "npm", detail: "Node.js 包管理" },
  { value: "pnpm", detail: "高效 Node.js 包管理" },
  { value: "pwd", detail: "显示当前目录" },
  { value: "rm", detail: "删除文件或目录" },
  { value: "rsync", detail: "同步文件和目录" },
  { value: "scp", detail: "通过 SSH 复制文件" },
  { value: "ssh", detail: "连接 SSH 主机" },
  { value: "systemctl", detail: "管理 systemd 单元" },
  { value: "tail", detail: "输出文件末尾" },
  { value: "tar", detail: "归档文件" },
  { value: "top", detail: "查看系统进程" },
  { value: "uname", detail: "显示系统信息" },
  { value: "vi", detail: "启动 Vi 编辑器" },
  { value: "vim", detail: "启动 Vim 编辑器" },
  { value: "wget", detail: "下载网络资源" },
  { value: "whoami", detail: "显示当前用户" },
];

const optionCatalog: Record<string, CatalogEntry[]> = {
  cargo: options(["--help", "--version", "--locked", "--offline", "-q"], "Cargo 选项"),
  curl: options(["--fail", "-H", "-I", "-L", "-o", "-X", "-d"], "curl 选项"),
  docker: options(["--help", "--version"], "Docker 选项"),
  find: options(["-maxdepth", "-mtime", "-name", "-size", "-type"], "find 条件"),
  git: options(["--help", "--no-pager", "--version", "-C"], "Git 全局选项"),
  grep: options(["--color=auto", "-E", "-F", "-i", "-n", "-r"], "grep 选项"),
  journalctl: options(["--no-pager", "--since", "-f", "-n", "-u"], "journalctl 选项"),
  ls: options(["--color=auto", "-R", "-a", "-h", "-l"], "ls 选项"),
  npm: options(["--help", "--silent", "--version"], "npm 选项"),
  pnpm: options(["--help", "--silent", "--version"], "pnpm 选项"),
  ssh: options(["-D", "-J", "-L", "-R", "-i", "-p", "-v"], "SSH 选项"),
  systemctl: options(["--no-pager", "--system", "--user"], "systemctl 选项"),
  tar: options(["-c", "-f", "-v", "-x", "-z"], "tar 选项"),
};

const argumentCatalog: Record<string, CatalogEntry[]> = {
  cargo: argumentsFor(["add", "build", "check", "clean", "clippy", "doc", "fmt", "run", "test", "update"], "Cargo 子命令"),
  docker: argumentsFor(["build", "compose", "exec", "images", "inspect", "logs", "ps", "pull", "push", "run"], "Docker 子命令"),
  git: argumentsFor(["add", "branch", "checkout", "commit", "diff", "fetch", "log", "pull", "push", "rebase", "restore", "stash", "status", "switch", "tag"], "Git 子命令"),
  npm: argumentsFor(["audit", "build", "install", "outdated", "publish", "run", "start", "test"], "npm 子命令"),
  pnpm: argumentsFor(["add", "audit", "build", "install", "remove", "run", "test", "update"], "pnpm 子命令"),
  systemctl: argumentsFor(["disable", "enable", "list-units", "reload", "restart", "start", "status", "stop"], "systemctl 操作"),
};

export function reduceTerminalCompletionInput(
  current: TerminalCompletionInputState,
  text: string,
): TerminalCompletionInputState {
  let line = current.line;
  let synchronized = current.synchronized;
  for (const character of text) {
    if (character === "\r" || character === "\n" || character === "\u0003" || character === "\u0004") {
      line = "";
      synchronized = true;
      continue;
    }
    if (!synchronized) continue;
    if (character === "\b" || character === "\u007f") {
      line = Array.from(line).slice(0, -1).join("");
      continue;
    }
    if (character === "\u0015") {
      line = "";
      continue;
    }
    if (character === "\u0017") {
      line = line.replace(/\S+\s*$/, "");
      continue;
    }
    if (character >= " " && character !== "\u007f") {
      if (Array.from(line).length >= MAX_COMPLETION_LINE_CHARACTERS) {
        line = "";
        synchronized = false;
      } else {
        line += character;
      }
      continue;
    }
    line = "";
    synchronized = false;
  }
  return { line, synchronized };
}

export function terminalCompletionSuggestions({
  line,
  preferences,
  history = [],
  quickCommands = [],
}: {
  line: string;
  preferences: TerminalCompletionPreferences;
  history?: readonly string[];
  quickCommands?: readonly TerminalCompletionQuickCommand[];
}): TerminalCompletionSuggestion[] {
  if (!preferences.enabled || !completionLineIsSafe(line)) return [];
  const tokenMatch = line.match(/\S*$/);
  const token = tokenMatch?.[0] ?? "";
  const beforeToken = line.slice(0, line.length - token.length);
  const tokens = line.trimStart().split(/\s+/).filter(Boolean);
  const command = tokens[0] ?? "";
  const typedCharacters = token.length || command.length;
  if (typedCharacters < preferences.triggerCharacters) return [];

  const candidates: TerminalCompletionSuggestion[] = [];
  if (preferences.quickCommands) {
    for (const quick of quickCommands) {
      const target = normalizeCandidateLine(quick.command);
      if (!target || !target.startsWith(line) || target === line) continue;
      candidates.push(suggestion(`quick:${quick.id}`, "quick", quick.label, "快速命令", target, line, false));
    }
  }
  if (preferences.history) {
    for (let index = 0; index < history.length; index += 1) {
      const target = normalizeCandidateLine(history[index]);
      if (!target || !target.startsWith(line) || target === line) continue;
      candidates.push(suggestion(`history:${index}:${target}`, "history", target, "历史命令", target, line, false));
    }
  }

  if (tokens.length <= 1 && !beforeToken.trim() && preferences.commandNames) {
    const indentation = line.slice(0, line.length - line.trimStart().length);
    for (const entry of commandCatalog) {
      const target = `${indentation}${entry.value}`;
      if (!target.startsWith(line)) continue;
      candidates.push(suggestion(`command:${entry.value}`, "command", entry.value, entry.detail, target, line, true));
    }
  }

  if (command && beforeToken.trim()) {
    if (preferences.commandOptions && (!token || token.startsWith("-"))) {
      for (const entry of optionCatalog[command] ?? []) {
        const target = `${beforeToken}${entry.value}`;
        if (!target.startsWith(line)) continue;
        candidates.push(suggestion(`option:${command}:${entry.value}`, "option", entry.value, entry.detail, target, line, true));
      }
    }
    if (preferences.commandArgs && !token.startsWith("-") && tokens.length <= 2) {
      for (const entry of argumentCatalog[command] ?? []) {
        const target = `${beforeToken}${entry.value}`;
        if (!target.startsWith(line)) continue;
        candidates.push(suggestion(`argument:${command}:${entry.value}`, "argument", entry.value, entry.detail, target, line, true));
      }
    }
  }

  const seenTargets = new Set<string>();
  return candidates
    .sort((left, right) => sourcePriority(left.source) - sourcePriority(right.source)
      || left.target.length - right.target.length
      || left.label.localeCompare(right.label))
    .filter((candidate) => {
      if (!candidate.appendText || seenTargets.has(candidate.target)) return false;
      seenTargets.add(candidate.target);
      return true;
    })
    .slice(0, MAX_COMPLETION_CANDIDATES);
}

export function terminalCompletionSourceLabel(source: TerminalCompletionSource): string {
  if (source === "command") return "命令";
  if (source === "option") return "选项";
  if (source === "argument") return "参数";
  if (source === "history") return "历史";
  return "Quick";
}

function completionLineIsSafe(line: string): boolean {
  return Boolean(line.trim())
    && line.length <= MAX_COMPLETION_LINE_CHARACTERS
    && !/[\u0000-\u001f\u007f'"`\\$(){}[\];|&<>]/.test(line);
}

function normalizeCandidateLine(value: unknown): string {
  if (typeof value !== "string") return "";
  const target = value.trim();
  return completionLineIsSafe(target) ? target : "";
}

function suggestion(
  id: string,
  source: TerminalCompletionSource,
  label: string,
  detail: string,
  target: string,
  line: string,
  completeWithSpace: boolean,
): TerminalCompletionSuggestion {
  const trailingSpace = completeWithSpace && !target.endsWith(" ") ? " " : "";
  const appendText = `${target.slice(line.length)}${trailingSpace}`;
  return { id, source, label, detail, target, appendText };
}

function options(values: string[], detail: string): CatalogEntry[] {
  return values.map((value) => ({ value, detail }));
}

function argumentsFor(values: string[], detail: string): CatalogEntry[] {
  return values.map((value) => ({ value, detail }));
}

function sourcePriority(source: TerminalCompletionSource): number {
  if (source === "quick") return 0;
  if (source === "history") return 1;
  if (source === "argument") return 2;
  if (source === "option") return 3;
  return 4;
}
