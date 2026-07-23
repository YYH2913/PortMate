export const OPENSSH_CONFIG_IMPORT_MAX_SOURCE_CHARS = 1_000_000;
export const OPENSSH_CONFIG_IMPORT_MAX_CANDIDATES = 256;

const MAX_CONFIG_LINES = 16_384;
const MAX_WARNINGS = 128;
const MAX_CANDIDATE_WARNINGS = 24;
const MAX_IDENTITY_FILES = 32;

export type OpenSshImportJump = {
  host: string;
  port: number;
  username: string;
};

export type OpenSshImportCandidate = {
  id: string;
  hostAlias: string;
  host: string;
  port: number;
  username: string;
  hostKeyAlias?: string;
  identityFiles: string[];
  keepaliveEnabled?: boolean;
  keepaliveIntervalSeconds?: number;
  keepaliveMaxMissed?: number;
  identitiesOnly?: boolean;
  forwardAgent?: boolean;
  jumps: OpenSshImportJump[];
  warnings: string[];
};

export type OpenSshConfigImportResult = {
  candidates: OpenSshImportCandidate[];
  warnings: string[];
  error: string | null;
};

type MutableCandidate = OpenSshImportCandidate & {
  defined: Set<string>;
};

type ParsedDirective = {
  keyword: string;
  values: string[];
};

export function parseOpenSshConfig(source: string): OpenSshConfigImportResult {
  if (source.length > OPENSSH_CONFIG_IMPORT_MAX_SOURCE_CHARS) {
    return {
      candidates: [],
      warnings: [],
      error: `OpenSSH 配置超过 ${OPENSSH_CONFIG_IMPORT_MAX_SOURCE_CHARS.toLocaleString()} 字符限制`,
    };
  }

  const warnings: string[] = [];
  const candidates = new Map<string, MutableCandidate>();
  const lines = source.replace(/^\uFEFF/, "").split(/\r\n?|\n/);
  let activeAliases: string[] = [];
  let inactiveConditionalBlock = false;

  const addWarning = (message: string) => {
    if (warnings.length < MAX_WARNINGS && !warnings.includes(message)) warnings.push(message);
  };
  const getCandidate = (alias: string) => {
    const existing = candidates.get(alias);
    if (existing) return existing;
    if (candidates.size >= OPENSSH_CONFIG_IMPORT_MAX_CANDIDATES) {
      addWarning(`最多导入 ${OPENSSH_CONFIG_IMPORT_MAX_CANDIDATES} 个字面 Host 条目，后续条目已跳过`);
      return null;
    }
    const candidate: MutableCandidate = {
      id: alias,
      hostAlias: alias,
      host: alias,
      port: 22,
      username: "",
      identityFiles: [],
      jumps: [],
      warnings: [],
      defined: new Set(),
    };
    candidates.set(alias, candidate);
    return candidate;
  };
  const addCandidateWarning = (candidate: MutableCandidate, lineNumber: number, message: string) => {
    const fullMessage = `Host ${candidate.hostAlias}，第 ${lineNumber} 行：${message}`;
    if (candidate.warnings.length < MAX_CANDIDATE_WARNINGS && !candidate.warnings.includes(message)) {
      candidate.warnings.push(message);
    }
    addWarning(fullMessage);
  };
  const withActiveCandidates = (lineNumber: number, apply: (candidate: MutableCandidate) => void) => {
    for (const alias of activeAliases) {
      const candidate = getCandidate(alias);
      if (candidate) apply(candidate);
    }
  };

  for (let index = 0; index < lines.length; index += 1) {
    const lineNumber = index + 1;
    if (lineNumber > MAX_CONFIG_LINES) {
      addWarning(`最多解析 ${MAX_CONFIG_LINES} 行，后续内容已跳过`);
      break;
    }
    const directive = parseDirective(lines[index]);
    if (!directive) continue;

    if (directive.keyword === "host") {
      activeAliases = [];
      inactiveConditionalBlock = false;
      if (!directive.values.length) {
        addWarning(`第 ${lineNumber} 行：Host 缺少名称，已跳过`);
        continue;
      }
      for (const alias of directive.values) {
        if (!isLiteralHost(alias)) {
          addWarning(`第 ${lineNumber} 行：Host ${alias} 不是字面条目，已跳过`);
          continue;
        }
        activeAliases.push(alias);
        getCandidate(alias);
      }
      inactiveConditionalBlock = activeAliases.length === 0;
      continue;
    }

    if (directive.keyword === "match") {
      activeAliases = [];
      inactiveConditionalBlock = true;
      addWarning(`第 ${lineNumber} 行：Match 条件块未导入`);
      continue;
    }
    if (directive.keyword === "include") {
      addWarning(`第 ${lineNumber} 行：Include 未读取外部文件`);
      continue;
    }
    if (!activeAliases.length) {
      if (!inactiveConditionalBlock) {
        addWarning(`第 ${lineNumber} 行：${directive.keyword} 不在字面 Host 条目中，未导入`);
      }
      continue;
    }

    switch (directive.keyword) {
      case "hostname": {
        const host = normalizeEndpointHost(directive.values[0]);
        withActiveCandidates(lineNumber, (candidate) => {
          if (!host) {
            addCandidateWarning(candidate, lineNumber, "HostName 不是可直接导入的字面地址");
            return;
          }
          setFirst(candidate, "host", host);
        });
        break;
      }
      case "user": {
        const username = normalizeValue(directive.values[0]);
        withActiveCandidates(lineNumber, (candidate) => {
          if (!username) {
            addCandidateWarning(candidate, lineNumber, "User 为空或包含动态标记");
            return;
          }
          setFirst(candidate, "username", username);
        });
        break;
      }
      case "port": {
        const port = parsePort(directive.values[0]);
        withActiveCandidates(lineNumber, (candidate) => {
          if (port === null) {
            addCandidateWarning(candidate, lineNumber, "Port 必须是 1 到 65535 的整数");
            return;
          }
          setFirst(candidate, "port", port);
        });
        break;
      }
      case "hostkeyalias": {
        const alias = normalizeValue(directive.values[0]);
        withActiveCandidates(lineNumber, (candidate) => {
          if (!alias) {
            addCandidateWarning(candidate, lineNumber, "HostKeyAlias 为空或包含动态标记");
            return;
          }
          setFirst(candidate, "hostKeyAlias", alias);
        });
        break;
      }
      case "identityfile": {
        const path = normalizeIdentityPath(directive.values[0]);
        withActiveCandidates(lineNumber, (candidate) => {
          if (!path) {
            addCandidateWarning(candidate, lineNumber, "IdentityFile 不是可直接导入的本地路径");
            return;
          }
          if (candidate.identityFiles.includes(path)) return;
          if (candidate.identityFiles.length >= MAX_IDENTITY_FILES) {
            addCandidateWarning(candidate, lineNumber, `最多保留 ${MAX_IDENTITY_FILES} 个 IdentityFile`);
            return;
          }
          candidate.identityFiles.push(path);
        });
        break;
      }
      case "serveraliveinterval": {
        const interval = parseInteger(directive.values[0], 0, 3_600);
        withActiveCandidates(lineNumber, (candidate) => {
          if (interval === null) {
            addCandidateWarning(candidate, lineNumber, "ServerAliveInterval 必须是 0 到 3600 的整数");
            return;
          }
          if (candidate.defined.has("keepaliveEnabled") || candidate.defined.has("keepaliveIntervalSeconds")) return;
          candidate.keepaliveEnabled = interval > 0;
          candidate.defined.add("keepaliveEnabled");
          if (interval > 0) {
            candidate.keepaliveIntervalSeconds = interval;
            candidate.defined.add("keepaliveIntervalSeconds");
          }
        });
        break;
      }
      case "serveralivecountmax": {
        const maxMissed = parseInteger(directive.values[0], 1, 20);
        withActiveCandidates(lineNumber, (candidate) => {
          if (maxMissed === null) {
            addCandidateWarning(candidate, lineNumber, "ServerAliveCountMax 必须是 1 到 20 的整数");
            return;
          }
          setFirst(candidate, "keepaliveMaxMissed", maxMissed);
        });
        break;
      }
      case "identitiesonly": {
        const value = parseBoolean(directive.values[0]);
        withActiveCandidates(lineNumber, (candidate) => {
          if (value === null) {
            addCandidateWarning(candidate, lineNumber, "IdentitiesOnly 仅支持 yes 或 no");
            return;
          }
          setFirst(candidate, "identitiesOnly", value);
        });
        break;
      }
      case "forwardagent": {
        const value = parseBoolean(directive.values[0]);
        withActiveCandidates(lineNumber, (candidate) => {
          if (value === null) {
            addCandidateWarning(candidate, lineNumber, "ForwardAgent 仅支持 yes 或 no");
            return;
          }
          setFirst(candidate, "forwardAgent", value);
        });
        break;
      }
      case "proxyjump": {
        const jumps = parseProxyJump(directive.values);
        withActiveCandidates(lineNumber, (candidate) => {
          if (jumps === null) {
            addCandidateWarning(candidate, lineNumber, "ProxyJump 仅支持逗号分隔的 [user@]host[:port] 字面地址");
            return;
          }
          if (candidate.defined.has("jumps")) return;
          candidate.jumps = jumps;
          candidate.defined.add("jumps");
        });
        break;
      }
      default:
        withActiveCandidates(lineNumber, (candidate) => {
          addCandidateWarning(candidate, lineNumber, `${directive.keyword} 未导入`);
        });
    }
  }

  return {
    candidates: [...candidates.values()].map(({ defined: _defined, ...candidate }) => candidate),
    warnings,
    error: null,
  };
}

function setFirst<T extends keyof OpenSshImportCandidate>(
  candidate: MutableCandidate,
  field: T,
  value: OpenSshImportCandidate[T],
) {
  if (candidate.defined.has(field)) return;
  (candidate as OpenSshImportCandidate)[field] = value;
  candidate.defined.add(field);
}

function parseDirective(line: string): ParsedDirective | null {
  const withoutComment = stripComment(line).trim();
  if (!withoutComment) return null;
  const match = withoutComment.match(/^([^\s=]+)(?:\s*=\s*|\s+)(.*)$/);
  if (!match) return { keyword: withoutComment.toLowerCase(), values: [] };
  return { keyword: match[1].toLowerCase(), values: tokenize(match[2]) };
}

function stripComment(line: string): string {
  let quote = "";
  let escaped = false;
  for (let index = 0; index < line.length; index += 1) {
    const character = line[index];
    if (escaped) {
      escaped = false;
      continue;
    }
    if (character === "\\") {
      escaped = true;
      continue;
    }
    if (quote) {
      if (character === quote) quote = "";
      continue;
    }
    if (character === '"' || character === "'") {
      quote = character;
      continue;
    }
    if (character === "#") return line.slice(0, index);
  }
  return line;
}

function tokenize(value: string): string[] {
  const tokens: string[] = [];
  let token = "";
  let quote = "";
  let escaped = false;
  const push = () => {
    if (token) tokens.push(token);
    token = "";
  };
  for (const character of value.trim()) {
    if (escaped) {
      token += character;
      escaped = false;
      continue;
    }
    if (character === "\\") {
      escaped = true;
      continue;
    }
    if (quote) {
      if (character === quote) quote = "";
      else token += character;
      continue;
    }
    if (character === '"' || character === "'") {
      quote = character;
      continue;
    }
    if (/\s/.test(character)) {
      push();
      continue;
    }
    token += character;
  }
  if (escaped) token += "\\";
  push();
  return tokens;
}

function isLiteralHost(value: string): boolean {
  const normalized = normalizeValue(value);
  return normalized !== null && !/[*!?]/.test(normalized);
}

function normalizeEndpointHost(value: string | undefined): string | null {
  const normalized = normalizeValue(value);
  if (!normalized || /[*!?]/.test(normalized)) return null;
  if (normalized.startsWith("[") && normalized.endsWith("]")) return normalized.slice(1, -1) || null;
  return normalized;
}

function normalizeValue(value: string | undefined): string | null {
  if (!value) return null;
  const normalized = value.trim();
  if (!normalized || normalized.includes("\0") || normalized.includes("%")) return null;
  return normalized;
}

function normalizeIdentityPath(value: string | undefined): string | null {
  const normalized = normalizeValue(value);
  if (!normalized || normalized.toLowerCase() === "none") return null;
  if (normalized.startsWith("~") && normalized !== "~" && !normalized.startsWith("~/")) return null;
  return normalized;
}

function parsePort(value: string | undefined): number | null {
  return parseInteger(value, 1, 65_535);
}

function parseInteger(value: string | undefined, min: number, max: number): number | null {
  if (!value || !/^\d+$/.test(value)) return null;
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < min || parsed > max) return null;
  return parsed;
}

function parseBoolean(value: string | undefined): boolean | null {
  switch (value?.toLowerCase()) {
    case "yes":
    case "true":
    case "on":
      return true;
    case "no":
    case "false":
    case "off":
      return false;
    default:
      return null;
  }
}

function parseProxyJump(values: string[]): OpenSshImportJump[] | null {
  if (values.length !== 1) return null;
  const raw = values[0];
  if (raw.toLowerCase() === "none") return [];
  if (!raw || raw.includes("%")) return null;
  const hops = raw.split(",");
  if (!hops.length || hops.some((hop) => !hop)) return null;
  const parsed = hops.map(parseProxyJumpHop);
  return parsed.every((hop): hop is OpenSshImportJump => hop !== null) ? parsed : null;
}

function parseProxyJumpHop(raw: string): OpenSshImportJump | null {
  const at = raw.lastIndexOf("@");
  const parsedUsername = at >= 0 ? normalizeValue(raw.slice(0, at)) : "";
  const endpoint = at >= 0 ? raw.slice(at + 1) : raw;
  if (at >= 0 && !parsedUsername) return null;
  const username = parsedUsername ?? "";

  const bracketed = endpoint.match(/^\[([^\]]+)\](?::(\d+))?$/);
  if (bracketed) {
    const host = normalizeEndpointHost(bracketed[1]);
    const port = bracketed[2] ? parsePort(bracketed[2]) : 22;
    return host && port !== null ? { host, port, username } : null;
  }

  const colon = endpoint.lastIndexOf(":");
  const hasPort = colon > 0 && endpoint.indexOf(":") === colon;
  const host = normalizeEndpointHost(hasPort ? endpoint.slice(0, colon) : endpoint);
  const port = hasPort ? parsePort(endpoint.slice(colon + 1)) : 22;
  return host && port !== null ? { host, port, username } : null;
}
