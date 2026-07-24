export const OPENSSH_CONFIG_IMPORT_MAX_SOURCE_CHARS = 1_000_000;
export const OPENSSH_CONFIG_IMPORT_MAX_CANDIDATES = 256;

const MAX_CONFIG_LINES = 16_384;
const MAX_WARNINGS = 128;
const MAX_CANDIDATE_WARNINGS = 24;
const MAX_IDENTITY_FILES = 32;
const MAX_FORWARDS = 64;
const GLOBAL_DEFAULT_HOST_ALIAS = "*";

export type OpenSshImportJump = {
  host: string;
  port: number;
  username: string;
};

export type OpenSshImportForward = {
  mode: "local" | "remote" | "dynamic";
  bindHost: string;
  bindPort: number;
  targetHost: string;
  targetPort: number;
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
  tcpKeepaliveEnabled?: boolean;
  identitiesOnly?: boolean;
  forwardAgent?: boolean;
  jumps: OpenSshImportJump[];
  forwards: OpenSshImportForward[];
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
  let activeGlobalDefaults = false;
  let inactiveConditionalBlock = false;
  const globalDefaults: MutableCandidate = {
    id: GLOBAL_DEFAULT_HOST_ALIAS,
    hostAlias: GLOBAL_DEFAULT_HOST_ALIAS,
    host: GLOBAL_DEFAULT_HOST_ALIAS,
    port: 22,
    username: "",
    identityFiles: [],
    jumps: [],
    forwards: [],
    warnings: [],
    defined: new Set(),
  };

  const addWarning = (message: string) => {
    if (warnings.length < MAX_WARNINGS && !warnings.includes(message)) warnings.push(message);
  };
  const addCandidateWarning = (candidate: MutableCandidate, lineNumber: number, message: string) => {
    const hostLabel = candidate === globalDefaults
      ? `Host ${GLOBAL_DEFAULT_HOST_ALIAS}`
      : `Host ${candidate.hostAlias}`;
    if (candidate !== globalDefaults
      && candidate.warnings.length < MAX_CANDIDATE_WARNINGS
      && !candidate.warnings.includes(message)) {
      candidate.warnings.push(message);
    }
    addWarning(`${hostLabel}，第 ${lineNumber} 行：${message}`);
  };
  const addForward = (
    candidate: MutableCandidate,
    forward: OpenSshImportForward,
    lineNumber: number | null,
    source = "转发",
  ) => {
    if (candidate.forwards.some((existing) => forwardKey(existing) === forwardKey(forward))) return;
    if (candidate.forwards.length >= MAX_FORWARDS) {
      if (lineNumber !== null) {
        addCandidateWarning(candidate, lineNumber, `${source}超过 ${MAX_FORWARDS} 条，后续未导入`);
      }
      return;
    }
    candidate.forwards.push({ ...forward });
  };
  const applyGlobalDefaults = (candidate: MutableCandidate, lineNumber: number | null) => {
    if (candidate === globalDefaults) return;
    if (globalDefaults.defined.has("host")) setFirst(candidate, "host", globalDefaults.host);
    if (globalDefaults.defined.has("port")) setFirst(candidate, "port", globalDefaults.port);
    if (globalDefaults.defined.has("username")) setFirst(candidate, "username", globalDefaults.username);
    if (globalDefaults.defined.has("hostKeyAlias")) {
      setFirst(candidate, "hostKeyAlias", globalDefaults.hostKeyAlias!);
    }
    if (globalDefaults.defined.has("keepaliveEnabled")) {
      setFirst(candidate, "keepaliveEnabled", globalDefaults.keepaliveEnabled!);
    }
    if (globalDefaults.defined.has("keepaliveIntervalSeconds")) {
      setFirst(candidate, "keepaliveIntervalSeconds", globalDefaults.keepaliveIntervalSeconds!);
    }
    if (globalDefaults.defined.has("keepaliveMaxMissed")) {
      setFirst(candidate, "keepaliveMaxMissed", globalDefaults.keepaliveMaxMissed!);
    }
    if (globalDefaults.defined.has("tcpKeepaliveEnabled")) {
      setFirst(candidate, "tcpKeepaliveEnabled", globalDefaults.tcpKeepaliveEnabled!);
    }
    if (globalDefaults.defined.has("identitiesOnly")) {
      setFirst(candidate, "identitiesOnly", globalDefaults.identitiesOnly!);
    }
    if (globalDefaults.defined.has("forwardAgent")) {
      setFirst(candidate, "forwardAgent", globalDefaults.forwardAgent!);
    }
    if (globalDefaults.defined.has("jumps")) {
      setFirst(candidate, "jumps", globalDefaults.jumps.map((jump) => ({ ...jump })));
    }
    for (const path of globalDefaults.identityFiles) {
      if (candidate.identityFiles.includes(path)) continue;
      if (candidate.identityFiles.length >= MAX_IDENTITY_FILES) {
        if (lineNumber !== null) {
          addCandidateWarning(candidate, lineNumber, `继承 Host * 的 IdentityFile 超过 ${MAX_IDENTITY_FILES} 个，后续未导入`);
        }
        break;
      }
      candidate.identityFiles.push(path);
    }
    for (const forward of globalDefaults.forwards) {
      addForward(candidate, forward, lineNumber, "继承 Host * 的转发");
    }
  };
  const applyGlobalDefaultsToExistingCandidates = (lineNumber: number) => {
    for (const candidate of candidates.values()) applyGlobalDefaults(candidate, lineNumber);
  };
  const getCandidate = (alias: string, lineNumber: number | null = null) => {
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
      forwards: [],
      warnings: [],
      defined: new Set(),
    };
    candidates.set(alias, candidate);
    applyGlobalDefaults(candidate, lineNumber);
    return candidate;
  };
  const withActiveCandidates = (lineNumber: number, apply: (candidate: MutableCandidate) => void) => {
    if (activeGlobalDefaults) {
      apply(globalDefaults);
      applyGlobalDefaultsToExistingCandidates(lineNumber);
      return;
    }
    for (const alias of activeAliases) {
      const candidate = getCandidate(alias, lineNumber);
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
      activeGlobalDefaults = false;
      inactiveConditionalBlock = false;
      if (!directive.values.length) {
        addWarning(`第 ${lineNumber} 行：Host 缺少名称，已跳过`);
        continue;
      }
      if (directive.values.length === 1 && directive.values[0] === GLOBAL_DEFAULT_HOST_ALIAS) {
        activeGlobalDefaults = true;
        continue;
      }
      for (const alias of directive.values) {
        if (!isLiteralHost(alias)) {
          addWarning(`第 ${lineNumber} 行：Host ${alias} 不是字面条目，已跳过`);
          continue;
        }
        activeAliases.push(alias);
        getCandidate(alias, lineNumber);
      }
      inactiveConditionalBlock = activeAliases.length === 0;
      continue;
    }

    if (directive.keyword === "match") {
      activeAliases = [];
      activeGlobalDefaults = false;
      inactiveConditionalBlock = true;
      addWarning(`第 ${lineNumber} 行：Match 条件块未导入`);
      continue;
    }
    if (directive.keyword === "include") {
      addWarning(`第 ${lineNumber} 行：Include 未读取外部文件`);
      continue;
    }
    if (!activeAliases.length && !activeGlobalDefaults) {
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
        const maxMissed = parseInteger(directive.values[0], 0, 20);
        withActiveCandidates(lineNumber, (candidate) => {
          if (maxMissed === null) {
            addCandidateWarning(candidate, lineNumber, "ServerAliveCountMax 必须是 0 到 20 的整数");
            return;
          }
          if (maxMissed === 0) {
            addCandidateWarning(candidate, lineNumber, "ServerAliveCountMax=0 会在首个保活探测前断开，PortMate 未导入该值");
            return;
          }
          setFirst(candidate, "keepaliveMaxMissed", maxMissed);
        });
        break;
      }
      case "tcpkeepalive": {
        const value = parseBoolean(directive.values[0]);
        withActiveCandidates(lineNumber, (candidate) => {
          if (value === null) {
            addCandidateWarning(candidate, lineNumber, "TCPKeepAlive 仅支持 yes 或 no");
            return;
          }
          setFirst(candidate, "tcpKeepaliveEnabled", value);
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
      case "localforward":
      case "remoteforward":
      case "dynamicforward": {
        const forward = parseOpenSshForward(directive.keyword, directive.values);
        withActiveCandidates(lineNumber, (candidate) => {
          if (!forward) {
            addCandidateWarning(candidate, lineNumber, `${directive.keyword} 仅支持安全的 TCP [bind_host:]port 和 host:port 字面地址`);
            return;
          }
          addForward(candidate, forward, lineNumber);
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

function forwardKey(forward: OpenSshImportForward): string {
  return [
    forward.mode,
    forward.bindHost,
    forward.bindPort,
    forward.targetHost,
    forward.targetPort,
  ].join("\0");
}

function parseOpenSshForward(
  keyword: string,
  values: string[],
): OpenSshImportForward | null {
  if (keyword === "dynamicforward") {
    const bind = values.length === 1
      ? parseOpenSshForwardBind(values[0], "127.0.0.1")
      : null;
    return bind ? {
      mode: "dynamic",
      bindHost: bind.host,
      bindPort: bind.port,
      targetHost: "",
      targetPort: 0,
    } : null;
  }

  const mode = keyword === "localforward"
    ? "local"
    : keyword === "remoteforward"
      ? "remote"
      : null;
  if (!mode || values.length !== 2) return null;

  const bind = parseOpenSshForwardBind(values[0], mode === "remote" ? "" : "127.0.0.1");
  const target = parseOpenSshForwardTarget(values[1]);
  if (!bind || !target) return null;
  return {
    mode,
    bindHost: bind.host,
    bindPort: bind.port,
    targetHost: target.host,
    targetPort: target.port,
  };
}

function parseOpenSshForwardBind(
  value: string | undefined,
  defaultHost: string,
): { host: string; port: number } | null {
  return parseOpenSshForwardEndpoint(value, defaultHost, true);
}

function parseOpenSshForwardTarget(
  value: string | undefined,
): { host: string; port: number } | null {
  return parseOpenSshForwardEndpoint(value, null, false);
}

function parseOpenSshForwardEndpoint(
  value: string | undefined,
  defaultHost: string | null,
  allowPortZero: boolean,
): { host: string; port: number } | null {
  if (!value || value.includes("%")) return null;
  const port = (raw: string) => parseInteger(raw, allowPortZero ? 0 : 1, 65_535);
  if (/^\d+$/.test(value)) {
    const parsedPort = port(value);
    return defaultHost !== null && parsedPort !== null
      ? { host: defaultHost, port: parsedPort }
      : null;
  }

  const bracketed = value.match(/^\[([^\]]+)]:(\d+)$/);
  if (bracketed) {
    const host = normalizeOpenSshForwardHost(bracketed[1]);
    const parsedPort = port(bracketed[2]);
    return host && parsedPort !== null ? { host, port: parsedPort } : null;
  }

  const separator = value.lastIndexOf(":");
  if (separator <= 0 || value.indexOf(":") !== separator) return null;
  const host = normalizeOpenSshForwardHost(value.slice(0, separator));
  const parsedPort = port(value.slice(separator + 1));
  return host && parsedPort !== null ? { host, port: parsedPort } : null;
}

function normalizeOpenSshForwardHost(value: string): string | null {
  const host = normalizeEndpointHost(value);
  return host && !/[\\/@[\]*?!]/.test(host) ? host : null;
}
