import { t } from "./i18n";
import type { ProxyKind } from "./types";
import { normalizeTerminalName, TERMINAL_PROFILE_BOUNDS } from "./terminal-settings-state";

export const PUTTY_SESSION_IMPORT_MAX_SOURCE_CHARS = 1_000_000;
export const PUTTY_SESSION_IMPORT_MAX_CANDIDATES = 256;

const MAX_CONFIG_LINES = 16_384;
const MAX_WARNINGS = 128;
const MAX_CANDIDATE_WARNINGS = 24;
const MAX_SESSION_SETTINGS = 512;
const MAX_FORWARDS = 64;
const MAX_FORWARDING_ENTRIES = 256;
const MAX_FORWARD_HOST_CHARACTERS = 255;
const PUTTY_REGISTRY_SESSION_PREFIX = "hkey_current_user\\software\\simontatham\\putty\\sessions\\";

export type PuttyProxyImport = {
  kind: ProxyKind;
  host: string;
  port: number;
  username: string;
};

export type PuttyForwardImport = {
  mode: "local" | "remote" | "dynamic";
  bindHost: string;
  bindPort: number;
  targetHost: string;
  targetPort: number;
};

export type PuttyTerminalImport = {
  term?: string;
  rows?: number;
  cols?: number;
  scrollback?: number;
};

type PuttyCandidateBase = {
  id: string;
  name: string;
  terminal?: PuttyTerminalImport;
  warnings: string[];
};

export type PuttyNetworkImportCandidate = PuttyCandidateBase & {
  kind: "ssh" | "telnet" | "tcp";
  host: string;
  port: number;
  username: string;
  proxy?: PuttyProxyImport;
  tryAgent?: boolean;
  forwardAgent?: boolean;
  keepaliveEnabled?: boolean;
  keepaliveIntervalSeconds?: number;
  keepaliveMaxMissed?: number;
  tcpKeepaliveEnabled?: boolean;
  forwards?: PuttyForwardImport[];
};

export type PuttySerialImportCandidate = PuttyCandidateBase & {
  kind: "serial";
  serial: {
    port: string;
    baudRate?: number;
    dataBits?: number;
    stopBits?: 1 | 2;
    parity?: "none" | "odd" | "even";
    flowControl?: "none" | "software" | "hardware";
  };
};

export type PuttySessionImportCandidate = PuttyNetworkImportCandidate | PuttySerialImportCandidate;

export type PuttySessionImportResult = {
  candidates: PuttySessionImportCandidate[];
  warnings: string[];
  error: string | null;
};

type RawPuttySession = {
  name: string;
  settings: Map<string, string>;
  lineNumber: number;
};

export function parsePuttySessions(source: string, sourceName = ""): PuttySessionImportResult {
  if (source.length > PUTTY_SESSION_IMPORT_MAX_SOURCE_CHARS) {
    return {
      candidates: [],
      warnings: [],
      error: t("putty-configuration-exceeds-the-character-limit", [PUTTY_SESSION_IMPORT_MAX_SOURCE_CHARS.toLocaleString()]),
    };
  }

  const warnings: string[] = [];
  const addWarning = (message: string) => {
    if (warnings.length < MAX_WARNINGS && !warnings.includes(message)) warnings.push(message);
  };
  const lines = source.replace(/^\uFEFF/, "").split(/\r\n?|\n/);
  const rawSessions = isRegistryExport(lines)
    ? parseRegistrySessions(lines, addWarning)
    : parseUnixSession(lines, sourceName, addWarning);
  const candidates: PuttySessionImportCandidate[] = [];

  for (const rawSession of rawSessions) {
    if (candidates.length >= PUTTY_SESSION_IMPORT_MAX_CANDIDATES) {
      addWarning(t("at-most-putty-sessions-can-be-imported-remaining-entries", [PUTTY_SESSION_IMPORT_MAX_CANDIDATES]));
      break;
    }
    const candidate = buildCandidate(rawSession, candidates.length + 1, addWarning);
    if (candidate) candidates.push(candidate);
  }

  if (!rawSessions.length && source.trim()) {
    addWarning(t("no-parseable-putty-session-configuration-found"));
  }

  return { candidates, warnings, error: null };
}

function isRegistryExport(lines: string[]): boolean {
  return lines.some((line) => /^\s*\[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\/i.test(line));
}

function parseRegistrySessions(lines: string[], addWarning: (message: string) => void): RawPuttySession[] {
  const sessions: RawPuttySession[] = [];
  let activeSession: RawPuttySession | null = null;

  for (let index = 0; index < lines.length; index += 1) {
    const lineNumber = index + 1;
    if (lineNumber > MAX_CONFIG_LINES) {
      addWarning(t("at-most-lines-can-be-parsed-remaining-content-was", [MAX_CONFIG_LINES]));
      break;
    }
    const line = lines[index].trim();
    const section = line.match(/^\[([^\]]+)]$/);
    if (section) {
      activeSession = createRegistrySession(section[1], lineNumber, sessions, addWarning);
      continue;
    }
    if (!activeSession) continue;
    const setting = parseRegistrySetting(line);
    if (setting) addSetting(activeSession, setting.key, setting.value, lineNumber, addWarning);
  }

  return sessions;
}

function createRegistrySession(
  section: string,
  lineNumber: number,
  sessions: RawPuttySession[],
  addWarning: (message: string) => void,
): RawPuttySession | null {
  const lower = section.toLowerCase();
  if (!lower.startsWith(PUTTY_REGISTRY_SESSION_PREFIX)) return null;
  const encodedName = section.slice(PUTTY_REGISTRY_SESSION_PREFIX.length);
  if (!encodedName || encodedName.includes("\\")) {
    addWarning(t("line-invalid-putty-registry-session-name-skipped", [lineNumber]));
    return null;
  }
  const name = decodeSessionName(encodedName);
  if (!name) {
    addWarning(t("line-invalid-putty-registry-session-name-skipped", [lineNumber]));
    return null;
  }
  if (isDefaultSessionName(name)) {
    addWarning(t("line-putty-default-settings-were-not-imported-as-a", [lineNumber]));
    return null;
  }
  if (sessions.length >= PUTTY_SESSION_IMPORT_MAX_CANDIDATES) {
    addWarning(t("at-most-putty-sessions-can-be-parsed-remaining-entries", [PUTTY_SESSION_IMPORT_MAX_CANDIDATES]));
    return null;
  }
  const session = { name, settings: new Map<string, string>(), lineNumber };
  sessions.push(session);
  return session;
}

function parseRegistrySetting(line: string): { key: string; value: string } | null {
  const match = line.match(/^"([^"\\]+)"\s*=\s*(.+)$/);
  if (!match) return null;
  const value = match[2].trim();
  if (/^dword:[0-9a-f]{1,8}$/i.test(value)) return { key: match[1], value: value.toLowerCase() };
  const decoded = decodeRegistryString(value);
  return decoded === null ? null : { key: match[1], value: decoded };
}

function decodeRegistryString(value: string): string | null {
  if (!value.startsWith('"')) return null;
  let decoded = "";
  let escaped = false;
  for (let index = 1; index < value.length; index += 1) {
    const character = value[index];
    if (escaped) {
      switch (character) {
        case "n": decoded += "\n"; break;
        case "r": decoded += "\r"; break;
        case "t": decoded += "\t"; break;
        default: decoded += character;
      }
      escaped = false;
      continue;
    }
    if (character === "\\") {
      escaped = true;
      continue;
    }
    if (character === '"') return value.slice(index + 1).trim() ? null : decoded;
    decoded += character;
  }
  return null;
}

function parseUnixSession(
  lines: string[],
  sourceName: string,
  addWarning: (message: string) => void,
): RawPuttySession[] {
  const name = sessionNameFromSource(sourceName);
  if (isDefaultSessionName(name)) {
    addWarning(t("putty-default-settings-were-not-imported-as-a-separate"));
    return [];
  }
  const session: RawPuttySession = { name, settings: new Map<string, string>(), lineNumber: 1 };
  for (let index = 0; index < lines.length; index += 1) {
    const lineNumber = index + 1;
    if (lineNumber > MAX_CONFIG_LINES) {
      addWarning(t("at-most-lines-can-be-parsed-remaining-content-was", [MAX_CONFIG_LINES]));
      break;
    }
    const line = lines[index].trim();
    if (!line || line.startsWith("#") || line.startsWith(";")) continue;
    const delimiter = line.indexOf("=");
    if (delimiter <= 0) continue;
    addSetting(session, line.slice(0, delimiter), line.slice(delimiter + 1), lineNumber, addWarning);
  }
  return session.settings.size ? [session] : [];
}

function addSetting(
  session: RawPuttySession,
  key: string,
  value: string,
  lineNumber: number,
  addWarning: (message: string) => void,
) {
  const normalizedKey = key.trim().toLowerCase();
  if (!normalizedKey) return;
  if (!session.settings.has(normalizedKey) && session.settings.size >= MAX_SESSION_SETTINGS) {
    addWarning(t("session-line-too-many-settings-remaining-entries-were-skipped", [session.name, lineNumber]));
    return;
  }
  session.settings.set(normalizedKey, value);
}

function buildCandidate(
  raw: RawPuttySession,
  index: number,
  addWarning: (message: string) => void,
): PuttySessionImportCandidate | null {
  const warnings: string[] = [];
  const addCandidateWarning = (message: string) => {
    if (warnings.length < MAX_CANDIDATE_WARNINGS && !warnings.includes(message)) warnings.push(message);
    addWarning(t("session-line", [raw.name, raw.lineNumber, message]));
  };
  const protocol = raw.settings.get("protocol")?.trim().toLowerCase() || "ssh";
  const kind = protocol === "raw" ? "tcp" : protocol;

  if (kind !== "ssh" && kind !== "telnet" && kind !== "tcp" && kind !== "serial") {
    addCandidateWarning(t("protocol-is-unsupported-skipped", [protocol]));
    return null;
  }

  const publicKeyFile = raw.settings.get("publickeyfile")?.trim();
  if (publicKeyFile) {
    addCandidateWarning(t("publickeyfile-not-imported-portmate-does-not-directly-read-putty"));
  }

  if (kind === "serial") {
    const port = normalizeSerialPort(raw.settings.get("serialline"));
    if (!port) {
      addCandidateWarning(t("serialline-is-empty-or-invalid-skipped"));
      return null;
    }
    const candidate: PuttySerialImportCandidate = {
      id: `putty-${index}-${raw.name}`,
      name: raw.name,
      kind: "serial",
      serial: { port },
      warnings,
    };
    applySerialSettings(candidate, raw.settings, addCandidateWarning);
    applyTerminalSettings(candidate, raw.settings, addCandidateWarning);
    return candidate;
  }

  const host = normalizeHost(raw.settings.get("hostname"));
  if (!host) {
    addCandidateWarning(t("hostname-is-empty-or-invalid-skipped"));
    return null;
  }
  const port = readNetworkPort(raw.settings.get("portnumber"), kind, addCandidateWarning);
  if (port === null) return null;
  const username = normalizeUsername(raw.settings.get("username"), "UserName", addCandidateWarning);
  const candidate: PuttyNetworkImportCandidate = {
    id: `putty-${index}-${raw.name}`,
    name: raw.name,
    kind,
    host,
    port,
    username,
    warnings,
  };
  applyProxySettings(candidate, raw.settings, addCandidateWarning);
  applyTcpKeepaliveSettings(candidate, raw.settings, addCandidateWarning);
  if (kind === "ssh") applySshSettings(candidate, raw.settings, addCandidateWarning);
  applyTerminalSettings(candidate, raw.settings, addCandidateWarning);
  return candidate;
}

function applyTcpKeepaliveSettings(
  candidate: PuttyNetworkImportCandidate,
  settings: Map<string, string>,
  addWarning: (message: string) => void,
) {
  if (!settings.has("tcpkeepalives")) return;
  const enabled = readBoolean(settings.get("tcpkeepalives"));
  if (enabled === null) {
    addWarning(t("tcpkeepalives-supports-only-0-or-1"));
    return;
  }
  candidate.tcpKeepaliveEnabled = enabled;
}

function applyTerminalSettings(
  candidate: PuttyCandidateBase,
  settings: Map<string, string>,
  addWarning: (message: string) => void,
) {
  const terminal: PuttyTerminalImport = {};
  const rawTerm = settings.get("terminaltype");
  if (rawTerm !== undefined) {
    const term = normalizeTerminalName(rawTerm);
    if (!term) addWarning(t("terminaltype-must-be-a-standard-terminal-name-within-64"));
    else terminal.term = term;
  }

  const numericSettings: Array<{
    key: string;
    field: keyof Omit<PuttyTerminalImport, "term">;
    min: number;
    max: number;
    label: string;
    description: string;
  }> = [
    {
      key: "termheight",
      field: "rows",
      min: TERMINAL_PROFILE_BOUNDS.rows.min,
      max: TERMINAL_PROFILE_BOUNDS.rows.max,
      label: "TermHeight",
      description: t("terminal-rows"),
    },
    {
      key: "termwidth",
      field: "cols",
      min: TERMINAL_PROFILE_BOUNDS.cols.min,
      max: TERMINAL_PROFILE_BOUNDS.cols.max,
      label: "TermWidth",
      description: t("terminal-columns"),
    },
    {
      key: "scrollbacklines",
      field: "scrollback",
      min: TERMINAL_PROFILE_BOUNDS.scrollback.min,
      max: TERMINAL_PROFILE_BOUNDS.scrollback.max,
      label: "ScrollbackLines",
      description: t("terminal-scrollback"),
    },
  ];
  for (const setting of numericSettings) {
    const rawValue = settings.get(setting.key);
    if (rawValue === undefined) continue;
    const value = parseIntegerInRange(rawValue, setting.min, setting.max);
    if (value === null) {
      addWarning(t("must-be-an-integer-from-to-not-imported", [setting.label, setting.min, setting.max, setting.description]));
      continue;
    }
    terminal[setting.field] = value;
  }
  if (Object.keys(terminal).length) candidate.terminal = terminal;
}

function applySshSettings(
  candidate: PuttyNetworkImportCandidate,
  settings: Map<string, string>,
  addWarning: (message: string) => void,
) {
  const tryAgent = readBoolean(settings.get("tryagent"));
  if (settings.has("tryagent")) {
    if (tryAgent === null) addWarning(t("tryagent-supports-only-0-or-1"));
    else candidate.tryAgent = tryAgent;
  }
  const forwardAgent = readBoolean(settings.get("agentfwd"));
  if (settings.has("agentfwd")) {
    if (forwardAgent === null) addWarning(t("agentfwd-supports-only-0-or-1"));
    else candidate.forwardAgent = forwardAgent;
  }
  applyKeepaliveSettings(candidate, settings, addWarning);
  applyForwardingSettings(candidate, settings, addWarning);
}

function applyKeepaliveSettings(
  candidate: PuttyNetworkImportCandidate,
  settings: Map<string, string>,
  addWarning: (message: string) => void,
) {
  const minutes = settings.get("pinginterval");
  const seconds = settings.get("pingintervalsecs");
  if (minutes === undefined && seconds === undefined) return;

  const parsedMinutes = parsePuttyInteger(minutes ?? "0");
  const parsedSeconds = parsePuttyInteger(seconds ?? "0");
  if (parsedMinutes === null || parsedSeconds === null) {
    addWarning(t("pinginterval-and-pingintervalsecs-must-be-non-negative-integers-ssh"));
    return;
  }
  const intervalSeconds = parsedMinutes * 60 + parsedSeconds;
  if (!Number.isSafeInteger(intervalSeconds) || intervalSeconds > 3_600) {
    addWarning(t("total-pinginterval-must-be-0-3600-seconds-ssh-keepalive"));
    return;
  }
  candidate.keepaliveEnabled = intervalSeconds > 0;
  if (intervalSeconds > 0) {
    candidate.keepaliveIntervalSeconds = intervalSeconds;
    // PuTTY sends SSH_MSG_IGNORE indefinitely instead of timing out on missed replies.
    candidate.keepaliveMaxMissed = 0;
  }
}

function applyForwardingSettings(
  candidate: PuttyNetworkImportCandidate,
  settings: Map<string, string>,
  addWarning: (message: string) => void,
) {
  const rawForwardings = settings.get("portforwardings");
  if (rawForwardings === undefined || !rawForwardings.trim()) return;

  const forwardingMap = parsePuttyForwardingMap(rawForwardings);
  if (!forwardingMap) {
    addWarning(t("invalid-portforwardings-format-port-forwarding-not-imported"));
    return;
  }
  if (forwardingMap.truncated) {
    addWarning(t("too-many-portforwardings-entries-checking-at-most", [MAX_FORWARDING_ENTRIES]));
  }

  const localPortAcceptAll = readForwardingBoolean(
    settings.get("localportacceptall"),
    "LocalPortAcceptAll",
    addWarning,
  );
  const remotePortAcceptAll = readForwardingBoolean(
    settings.get("remoteportacceptall"),
    "RemotePortAcceptAll",
    addWarning,
  );
  const forwards: PuttyForwardImport[] = [];
  let warnedLocalAcceptAllMapping = false;
  let warnedLimit = false;

  for (let index = 0; index < forwardingMap.entries.length; index += 1) {
    if (forwards.length >= MAX_FORWARDS) {
      if (!warnedLimit) {
        addWarning(t("at-most-portforwardings-entries-can-be-imported-remaining-entries", [MAX_FORWARDS]));
        warnedLimit = true;
      }
      break;
    }
    const parsed = parsePuttyForwarding(
      forwardingMap.entries[index],
      localPortAcceptAll,
      remotePortAcceptAll,
    );
    if ("error" in parsed) {
      addWarning(t("portforwardings-entry", [index + 1, parsed.error]));
      continue;
    }
    if (forwards.some((existing) => puttyForwardKey(existing) === puttyForwardKey(parsed.forward))) {
      continue;
    }
    forwards.push(parsed.forward);
    if (parsed.localAcceptAllMapped && !warnedLocalAcceptAllMapping) {
      addWarning(t("localportacceptall-1-mapped-to-0-0-0-0-add"));
      warnedLocalAcceptAllMapping = true;
    }
  }

  candidate.forwards = forwards;
}

function readForwardingBoolean(
  value: string | undefined,
  setting: string,
  addWarning: (message: string) => void,
): boolean {
  if (value === undefined) return false;
  const parsed = readBoolean(value);
  if (parsed !== null) return parsed;
  addWarning(t("supports-only-boolean-values-treated-as-disabled", [setting]));
  return false;
}

type PuttyForwardingMapEntry = {
  key: string;
  value: string;
};

type PuttyForwardingMap = {
  entries: PuttyForwardingMapEntry[];
  truncated: boolean;
};

function parsePuttyForwardingMap(value: string): PuttyForwardingMap | null {
  const entries: PuttyForwardingMapEntry[] = [];
  let key = "";
  let entryValue = "";
  let readingValue = false;
  let escaped = false;
  let truncated = false;

  const commit = () => {
    if (!key) return false;
    if (entries.length < MAX_FORWARDING_ENTRIES) {
      entries.push({ key, value: entryValue });
    } else {
      truncated = true;
    }
    key = "";
    entryValue = "";
    readingValue = false;
    return true;
  };

  for (const character of value) {
    if (escaped) {
      if (readingValue) entryValue += character;
      else key += character;
      escaped = false;
      continue;
    }
    if (character === "\\") {
      escaped = true;
      continue;
    }
    if (character === ",") {
      if (!commit()) return null;
      continue;
    }
    if (character === "=" && !readingValue) {
      readingValue = true;
      continue;
    }
    if (readingValue) entryValue += character;
    else key += character;
  }

  if (escaped) return null;
  if (key || entryValue || readingValue) {
    if (!commit()) return null;
  }
  return { entries, truncated };
}

function parsePuttyForwarding(
  entry: PuttyForwardingMapEntry,
  localPortAcceptAll: boolean,
  remotePortAcceptAll: boolean,
): { forward: PuttyForwardImport; localAcceptAllMapped: boolean } | { error: string } {
  let key = entry.key;
  let addressFamily = "A";
  if (key[0] === "A" || key[0] === "4" || key[0] === "6") {
    addressFamily = key[0];
    key = key.slice(1);
  }
  if (addressFamily !== "A") {
    return { error: t("forced-ipv4-ipv6-address-family-not-imported") };
  }

  const type = key[0];
  const source = key.slice(1);
  if (type !== "L" && type !== "R" && type !== "D") {
    return { error: t("forwarding-type-must-be-l-r-or-d") };
  }
  if (!source) return { error: t("missing-source-port") };

  const dynamic = type === "D" || (type === "L" && entry.value === "D");
  if (type === "D" && entry.value) {
    return { error: t("dynamic-forwarding-cannot-contain-a-target-address") };
  }
  const mode = dynamic ? "dynamic" : type === "L" ? "local" : "remote";
  const defaultBindHost = mode === "remote"
    ? (remotePortAcceptAll ? "" : "localhost")
    : (localPortAcceptAll ? "0.0.0.0" : "127.0.0.1");
  const bind = parsePuttyForwardEndpoint(source, defaultBindHost);
  if (!bind) {
    return { error: t("only-literal-tcp-listen-addresses-and-ports-are-supported") };
  }

  if (mode === "dynamic") {
    return {
      forward: {
        mode,
        bindHost: bind.host,
        bindPort: bind.port,
        targetHost: "",
        targetPort: 0,
      },
      localAcceptAllMapped: localPortAcceptAll && /^\d+$/.test(source),
    };
  }

  const target = parsePuttyForwardEndpoint(entry.value, null);
  if (!target) {
    return { error: t("only-literal-tcp-target-host-port-is-supported") };
  }
  return {
    forward: {
      mode,
      bindHost: bind.host,
      bindPort: bind.port,
      targetHost: target.host,
      targetPort: target.port,
    },
    localAcceptAllMapped: localPortAcceptAll && mode === "local" && /^\d+$/.test(source),
  };
}

function parsePuttyForwardEndpoint(
  value: string,
  defaultHost: string | null,
): { host: string; port: number } | null {
  if (!value || value.trim() !== value || value.includes("%") || /[\0\s]/.test(value)) return null;
  if (/^\d+$/.test(value)) {
    const port = parsePort(value);
    return defaultHost !== null && port !== null ? { host: defaultHost, port } : null;
  }

  const bracketed = value.match(/^\[([^\[\]]+)]:(\d+)$/);
  if (bracketed) {
    const host = normalizePuttyForwardHost(bracketed[1]);
    const port = parsePort(bracketed[2]);
    return host && port !== null ? { host, port } : null;
  }

  const separator = value.lastIndexOf(":");
  if (separator <= 0 || value.indexOf(":") !== separator) return null;
  const host = normalizePuttyForwardHost(value.slice(0, separator));
  const port = parsePort(value.slice(separator + 1));
  return host && port !== null ? { host, port } : null;
}

function normalizePuttyForwardHost(value: string): string | null {
  const host = normalizeText(value);
  if (!host || host.length > MAX_FORWARD_HOST_CHARACTERS) return null;
  return /[\\/@[\]*?!,=]/.test(host) ? null : host;
}

function puttyForwardKey(forward: PuttyForwardImport): string {
  return [
    forward.mode,
    forward.bindHost,
    forward.bindPort,
    forward.targetHost,
    forward.targetPort,
  ].join("\0");
}

function applyProxySettings(
  candidate: PuttyNetworkImportCandidate,
  settings: Map<string, string>,
  addWarning: (message: string) => void,
) {
  const password = settings.get("proxypassword")?.trim();
  if (password) addWarning(t("proxypassword-not-imported-enter-the-proxy-password-again-in"));
  const rawMethod = settings.get("proxymethod");
  if (rawMethod === undefined) return;
  const method = parsePuttyInteger(rawMethod);
  if (method === null || method < 0) {
    addWarning(t("proxymethod-must-be-a-valid-integer-proxy-not-imported"));
    return;
  }
  if (method === 0) return;
  const kind = method === 2 ? "socks5" : method === 3 ? "http-connect" : null;
  if (!kind) {
    addWarning(t("proxymethod-is-unsupported-proxy-not-imported", [method]));
    return;
  }
  const host = normalizeHost(settings.get("proxyhost"));
  const port = parsePort(settings.get("proxyport"));
  if (!host || port === null) {
    addWarning(t("invalid-proxyhost-or-proxyport-proxy-not-imported"));
    return;
  }
  candidate.proxy = {
    kind,
    host,
    port,
    username: normalizeUsername(settings.get("proxyusername"), "ProxyUsername", addWarning),
  };
}

function applySerialSettings(
  candidate: PuttySerialImportCandidate,
  settings: Map<string, string>,
  addWarning: (message: string) => void,
) {
  const speed = parseIntegerInRange(settings.get("serialspeed"), 1, 4_000_000);
  if (settings.has("serialspeed")) {
    if (speed === null) addWarning(t("serialspeed-must-be-an-integer-from-1-to-4000000"));
    else candidate.serial.baudRate = speed;
  }
  const dataBits = parseIntegerInRange(settings.get("serialdatabits"), 5, 8);
  if (settings.has("serialdatabits")) {
    if (dataBits === null) addWarning(t("serialdatabits-supports-only-5-8"));
    else candidate.serial.dataBits = dataBits;
  }

  const stopHalfbits = parsePuttyInteger(settings.get("serialstophalfbits"));
  if (settings.has("serialstophalfbits")) {
    if (stopHalfbits === 2) candidate.serial.stopBits = 1;
    else if (stopHalfbits === 4) candidate.serial.stopBits = 2;
    else if (stopHalfbits === 3) addWarning(t("serialstophalfbits-3-means-1-5-stop-bits-not-imported"));
    else addWarning(t("serialstophalfbits-supports-only-2-or-4"));
  }

  const parity = parsePuttyInteger(settings.get("serialparity"));
  if (settings.has("serialparity")) {
    if (parity === 0) candidate.serial.parity = "none";
    else if (parity === 1) candidate.serial.parity = "odd";
    else if (parity === 2) candidate.serial.parity = "even";
    else addWarning(t("serialparity-mark-and-space-modes-not-imported"));
  }

  const flowControl = parsePuttyInteger(settings.get("serialflowcontrol"));
  if (settings.has("serialflowcontrol")) {
    if (flowControl === 0) candidate.serial.flowControl = "none";
    else if (flowControl === 1) candidate.serial.flowControl = "software";
    else if (flowControl === 2) candidate.serial.flowControl = "hardware";
    else addWarning(t("serialflowcontrol-dsr-dtr-mode-not-imported"));
  }
}

function readNetworkPort(
  value: string | undefined,
  kind: PuttyNetworkImportCandidate["kind"],
  addWarning: (message: string) => void,
): number | null {
  const defaultPort = kind === "ssh" ? 22 : kind === "telnet" ? 23 : null;
  if (value === undefined || !value.trim()) {
    if (defaultPort !== null) return defaultPort;
    addWarning(t("raw-session-is-missing-portnumber-skipped"));
    return null;
  }
  const port = parsePort(value);
  if (port === null) {
    addWarning(t("portnumber-must-be-an-integer-from-1-to-65535"));
    return null;
  }
  return port;
}

function normalizeHost(value: string | undefined): string | null {
  const normalized = normalizeText(value);
  if (!normalized || /\s/.test(normalized)) return null;
  if (normalized.startsWith("[") && normalized.endsWith("]")) return normalized.slice(1, -1) || null;
  return normalized;
}

function normalizeSerialPort(value: string | undefined): string | null {
  const normalized = normalizeText(value);
  return normalized && !/[\r\n]/.test(normalized) ? normalized : null;
}

function normalizeUsername(
  value: string | undefined,
  label: string,
  addWarning: (message: string) => void,
): string {
  if (value === undefined || !value.trim()) return "";
  const normalized = normalizeText(value);
  if (!normalized || /\s/.test(normalized)) {
    addWarning(t("is-empty-or-invalid-not-imported", [label]));
    return "";
  }
  return normalized;
}

function normalizeText(value: string | undefined): string | null {
  if (value === undefined) return null;
  const normalized = value.trim();
  if (!normalized || /[\0-\x1f\x7f]/.test(normalized)) return null;
  return normalized;
}

function parsePort(value: string | undefined): number | null {
  return parseIntegerInRange(value, 1, 65_535);
}

function parseIntegerInRange(value: string | undefined, min: number, max: number): number | null {
  const parsed = parsePuttyInteger(value);
  return parsed !== null && parsed >= min && parsed <= max ? parsed : null;
}

function parsePuttyInteger(value: string | undefined): number | null {
  if (!value) return null;
  const normalized = value.trim();
  const dword = normalized.match(/^dword:([0-9a-f]{1,8})$/i);
  const parsed = dword ? Number.parseInt(dword[1], 16) : /^\d+$/.test(normalized) ? Number(normalized) : NaN;
  return Number.isSafeInteger(parsed) ? parsed : null;
}

function readBoolean(value: string | undefined): boolean | null {
  const numeric = parsePuttyInteger(value);
  if (numeric === 0) return false;
  if (numeric === 1) return true;
  switch (value?.trim().toLowerCase()) {
    case "true":
    case "yes":
      return true;
    case "false":
    case "no":
      return false;
    default:
      return null;
  }
}

function sessionNameFromSource(sourceName: string): string {
  const base = sourceName.trim().split(/[\\/]/).pop() ?? "";
  const withoutExtension = base.replace(/\.(?:ini|reg|session|txt)$/i, "");
  return decodeSessionName(withoutExtension) || t("putty-session");
}

function decodeSessionName(value: string): string | null {
  if (!value) return null;
  try {
    return normalizeText(decodeURIComponent(value));
  } catch {
    return normalizeText(value);
  }
}

function isDefaultSessionName(value: string): boolean {
  return value.trim().toLowerCase() === "default settings";
}
