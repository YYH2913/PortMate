import { t } from "./i18n";
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
      error: t("openssh-configuration-exceeds-the-character-limit", [OPENSSH_CONFIG_IMPORT_MAX_SOURCE_CHARS.toLocaleString()]),
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
    addWarning(t("line", [hostLabel, lineNumber, message]));
  };
  const addForward = (
    candidate: MutableCandidate,
    forward: OpenSshImportForward,
    lineNumber: number | null,
    source = t("forwards-2"),
  ) => {
    if (candidate.forwards.some((existing) => forwardKey(existing) === forwardKey(forward))) return;
    if (candidate.forwards.length >= MAX_FORWARDS) {
      if (lineNumber !== null) {
        addCandidateWarning(candidate, lineNumber, t("exceeds-entries-remaining-entries-were-not-imported", [source, MAX_FORWARDS]));
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
          addCandidateWarning(candidate, lineNumber, t("inherited-host-identityfile-entries-exceed-remaining-entries-were-not", [MAX_IDENTITY_FILES]));
        }
        break;
      }
      candidate.identityFiles.push(path);
    }
    for (const forward of globalDefaults.forwards) {
      addForward(candidate, forward, lineNumber, t("inherited-host-forwards"));
    }
  };
  const applyGlobalDefaultsToExistingCandidates = (lineNumber: number) => {
    for (const candidate of candidates.values()) applyGlobalDefaults(candidate, lineNumber);
  };
  const getCandidate = (alias: string, lineNumber: number | null = null) => {
    const existing = candidates.get(alias);
    if (existing) return existing;
    if (candidates.size >= OPENSSH_CONFIG_IMPORT_MAX_CANDIDATES) {
      addWarning(t("at-most-literal-host-entries-can-be-imported-remaining", [OPENSSH_CONFIG_IMPORT_MAX_CANDIDATES]));
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
      addWarning(t("at-most-lines-can-be-parsed-remaining-content-was", [MAX_CONFIG_LINES]));
      break;
    }
    const directive = parseDirective(lines[index]);
    if (!directive) continue;

    if (directive.keyword === "host") {
      activeAliases = [];
      activeGlobalDefaults = false;
      inactiveConditionalBlock = false;
      if (!directive.values.length) {
        addWarning(t("line-host-name-is-missing-skipped", [lineNumber]));
        continue;
      }
      if (directive.values.length === 1 && directive.values[0] === GLOBAL_DEFAULT_HOST_ALIAS) {
        activeGlobalDefaults = true;
        continue;
      }
      for (const alias of directive.values) {
        if (!isLiteralHost(alias)) {
          addWarning(t("line-host-is-not-a-literal-entry-skipped", [lineNumber, alias]));
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
      addWarning(t("line-conditional-match-block-not-imported", [lineNumber]));
      continue;
    }
    if (directive.keyword === "include") {
      addWarning(t("line-external-include-file-not-read", [lineNumber]));
      continue;
    }
    if (!activeAliases.length && !activeGlobalDefaults) {
      if (!inactiveConditionalBlock) {
        addWarning(t("line-is-outside-a-literal-host-entry-not-imported", [lineNumber, directive.keyword]));
      }
      continue;
    }

    switch (directive.keyword) {
      case "hostname": {
        const host = normalizeEndpointHost(directive.values[0]);
        withActiveCandidates(lineNumber, (candidate) => {
          if (!host) {
            addCandidateWarning(candidate, lineNumber, t("hostname-is-not-a-directly-importable-literal-address"));
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
            addCandidateWarning(candidate, lineNumber, t("user-is-empty-or-contains-dynamic-tokens"));
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
            addCandidateWarning(candidate, lineNumber, t("port-must-be-an-integer-from-1-to-65535"));
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
            addCandidateWarning(candidate, lineNumber, t("hostkeyalias-is-empty-or-contains-dynamic-tokens"));
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
            addCandidateWarning(candidate, lineNumber, t("identityfile-is-not-a-directly-importable-local-path"));
            return;
          }
          if (candidate.identityFiles.includes(path)) return;
          if (candidate.identityFiles.length >= MAX_IDENTITY_FILES) {
            addCandidateWarning(candidate, lineNumber, t("at-most-identityfile-entries-are-retained", [MAX_IDENTITY_FILES]));
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
            addCandidateWarning(candidate, lineNumber, t("serveraliveinterval-must-be-an-integer-from-0-to-3600"));
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
            addCandidateWarning(candidate, lineNumber, t("serveralivecountmax-must-be-an-integer-from-0-to-20"));
            return;
          }
          if (maxMissed === 0) {
            addCandidateWarning(candidate, lineNumber, t("serveralivecountmax-0-disconnects-before-the-first-keepalive-probe-portmate"));
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
            addCandidateWarning(candidate, lineNumber, t("tcpkeepalive-supports-only-yes-or-no"));
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
            addCandidateWarning(candidate, lineNumber, t("identitiesonly-supports-only-yes-or-no"));
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
            addCandidateWarning(candidate, lineNumber, t("forwardagent-supports-only-yes-or-no"));
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
            addCandidateWarning(candidate, lineNumber, t("proxyjump-supports-only-comma-separated-literal-user-host-port"));
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
            addCandidateWarning(candidate, lineNumber, t("supports-only-safe-literal-tcp-bind-host-port-and", [directive.keyword]));
            return;
          }
          addForward(candidate, forward, lineNumber);
        });
        break;
      }
      default:
        withActiveCandidates(lineNumber, (candidate) => {
          addCandidateWarning(candidate, lineNumber, t("not-imported", [directive.keyword]));
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
