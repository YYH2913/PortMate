export const SHELL_SESSION_IMPORT_MAX_SOURCE_CHARS = 1_000_000;
export const SHELL_SESSION_IMPORT_MAX_CANDIDATES = 256;

const MAX_SHELL_LINES = 16_384;
const MAX_WARNINGS = 128;
const KNOWN_WINDOWS_SHELLS = new Set(["cmd", "cmd.exe", "powershell", "powershell.exe", "pwsh", "pwsh.exe"]);
const NON_INTERACTIVE_SHELLS = new Set(["false", "nologin", "sync"]);

export type ShellSessionImportCandidate = {
  id: string;
  name: string;
  program: string;
  args: string[];
  warnings: string[];
};

export type ShellSessionImportResult = {
  candidates: ShellSessionImportCandidate[];
  warnings: string[];
  error: string | null;
};

export function parseShellSessions(source: string): ShellSessionImportResult {
  if (source.length > SHELL_SESSION_IMPORT_MAX_SOURCE_CHARS) {
    return {
      candidates: [],
      warnings: [],
      error: `Shell 列表超过 ${SHELL_SESSION_IMPORT_MAX_SOURCE_CHARS.toLocaleString()} 字符限制`,
    };
  }

  const warnings: string[] = [];
  const candidates: ShellSessionImportCandidate[] = [];
  const seenPrograms = new Set<string>();
  const addWarning = (message: string) => {
    if (warnings.length < MAX_WARNINGS && !warnings.includes(message)) warnings.push(message);
  };
  const lines = source.replace(/^\uFEFF/, "").split(/\r\n?|\n/);

  for (let index = 0; index < lines.length; index += 1) {
    const lineNumber = index + 1;
    if (lineNumber > MAX_SHELL_LINES) {
      addWarning(`最多解析 ${MAX_SHELL_LINES} 行，后续内容已跳过`);
      break;
    }
    const rawValue = stripComment(lines[index]);
    if (!rawValue) continue;
    const program = normalizeShellProgram(rawValue);
    if (!program) {
      addWarning(`第 ${lineNumber} 行：不是可直接导入的 Shell 路径`);
      continue;
    }
    const basename = shellBasename(program);
    if (NON_INTERACTIVE_SHELLS.has(basename.toLowerCase())) {
      addWarning(`第 ${lineNumber} 行：${basename} 不是交互式 Shell，已跳过`);
      continue;
    }
    const key = shellProgramKey(program);
    if (seenPrograms.has(key)) {
      addWarning(`第 ${lineNumber} 行：${program} 重复，已跳过`);
      continue;
    }
    if (candidates.length >= SHELL_SESSION_IMPORT_MAX_CANDIDATES) {
      addWarning(`最多导入 ${SHELL_SESSION_IMPORT_MAX_CANDIDATES} 个 Shell 会话，后续条目已跳过`);
      break;
    }
    seenPrograms.add(key);
    candidates.push({
      id: `shell-${candidates.length + 1}-${program}`,
      name: shellName(basename),
      program,
      args: [],
      warnings: [],
    });
  }

  return { candidates, warnings, error: null };
}

function stripComment(line: string): string {
  const trimmed = line.trim();
  if (!trimmed || trimmed.startsWith("#")) return "";
  const commentStart = trimmed.search(/\s+#/);
  return (commentStart >= 0 ? trimmed.slice(0, commentStart) : trimmed).trim();
}

function normalizeShellProgram(value: string): string | null {
  const quoted = value.length >= 2 && value.startsWith('"') && value.endsWith('"');
  const unquoted = quoted
    ? value.slice(1, -1)
    : value;
  const program = quoted ? unquoted : unquoted.trim();
  if (!program || /[\0-\x1f\x7f]/.test(program) || /[|;&`$<>]/.test(program) || (!quoted && /\s/.test(program))) return null;
  if (KNOWN_WINDOWS_SHELLS.has(program.toLowerCase())) return program;
  if (isAbsolutePosixPath(program) || isAbsoluteWindowsPath(program) || isUncPath(program)) return program;
  return null;
}

function isAbsolutePosixPath(program: string): boolean {
  const segments = program.split("/").filter(Boolean);
  return program.startsWith("/")
    && !program.endsWith("/")
    && segments.length > 0
    && !hasTraversalSegment(segments);
}

function isAbsoluteWindowsPath(program: string): boolean {
  const remainder = program.slice(3);
  const segments = remainder.split(/[\\/]/).filter(Boolean);
  return /^[a-z]:[\\/]/i.test(program)
    && Boolean(remainder)
    && !/[\\/]$/.test(program)
    && segments.length > 0
    && !hasTraversalSegment(segments);
}

function isUncPath(program: string): boolean {
  const segments = program.split(/\\+/).filter(Boolean);
  return program.startsWith("\\\\")
    && !/\\$/.test(program)
    && segments.length >= 3
    && !hasTraversalSegment(segments);
}

function hasTraversalSegment(segments: string[]): boolean {
  return segments.some((segment) => segment === "." || segment === "..");
}

function shellBasename(program: string): string {
  return program.replaceAll("\\", "/").split("/").filter(Boolean).at(-1) ?? program;
}

function shellName(basename: string): string {
  return basename.replace(/\.exe$/i, "") || basename;
}

function shellProgramKey(program: string): string {
  return isAbsoluteWindowsPath(program) || isUncPath(program) || KNOWN_WINDOWS_SHELLS.has(program.toLowerCase())
    ? program.toLowerCase()
    : program;
}
