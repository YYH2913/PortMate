import type { ProxyKind } from "./types";

export const PUTTY_SESSION_IMPORT_MAX_SOURCE_CHARS = 1_000_000;
export const PUTTY_SESSION_IMPORT_MAX_CANDIDATES = 256;

const MAX_CONFIG_LINES = 16_384;
const MAX_WARNINGS = 128;
const MAX_CANDIDATE_WARNINGS = 24;
const MAX_SESSION_SETTINGS = 512;
const PUTTY_REGISTRY_SESSION_PREFIX = "hkey_current_user\\software\\simontatham\\putty\\sessions\\";

export type PuttyProxyImport = {
  kind: ProxyKind;
  host: string;
  port: number;
  username: string;
};

type PuttyCandidateBase = {
  id: string;
  name: string;
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
      error: `PuTTY 配置超过 ${PUTTY_SESSION_IMPORT_MAX_SOURCE_CHARS.toLocaleString()} 字符限制`,
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
      addWarning(`最多导入 ${PUTTY_SESSION_IMPORT_MAX_CANDIDATES} 个 PuTTY 会话，后续条目已跳过`);
      break;
    }
    const candidate = buildCandidate(rawSession, candidates.length + 1, addWarning);
    if (candidate) candidates.push(candidate);
  }

  if (!rawSessions.length && source.trim()) {
    addWarning("没有找到可解析的 PuTTY 会话配置");
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
      addWarning(`最多解析 ${MAX_CONFIG_LINES} 行，后续内容已跳过`);
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
    addWarning(`第 ${lineNumber} 行：PuTTY 注册表会话名称无效，已跳过`);
    return null;
  }
  const name = decodeSessionName(encodedName);
  if (!name) {
    addWarning(`第 ${lineNumber} 行：PuTTY 注册表会话名称无效，已跳过`);
    return null;
  }
  if (isDefaultSessionName(name)) {
    addWarning(`第 ${lineNumber} 行：PuTTY Default Settings 未作为独立会话导入`);
    return null;
  }
  if (sessions.length >= PUTTY_SESSION_IMPORT_MAX_CANDIDATES) {
    addWarning(`最多解析 ${PUTTY_SESSION_IMPORT_MAX_CANDIDATES} 个 PuTTY 会话，后续条目已跳过`);
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
    addWarning("PuTTY Default Settings 未作为独立会话导入");
    return [];
  }
  const session: RawPuttySession = { name, settings: new Map<string, string>(), lineNumber: 1 };
  for (let index = 0; index < lines.length; index += 1) {
    const lineNumber = index + 1;
    if (lineNumber > MAX_CONFIG_LINES) {
      addWarning(`最多解析 ${MAX_CONFIG_LINES} 行，后续内容已跳过`);
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
    addWarning(`会话 ${session.name}，第 ${lineNumber} 行：配置项过多，后续项已跳过`);
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
    addWarning(`会话 ${raw.name}，第 ${raw.lineNumber} 行：${message}`);
  };
  const protocol = raw.settings.get("protocol")?.trim().toLowerCase() || "ssh";
  const kind = protocol === "raw" ? "tcp" : protocol;

  if (kind !== "ssh" && kind !== "telnet" && kind !== "tcp" && kind !== "serial") {
    addCandidateWarning(`Protocol=${protocol} 不受支持，已跳过`);
    return null;
  }

  const publicKeyFile = raw.settings.get("publickeyfile")?.trim();
  if (publicKeyFile) {
    addCandidateWarning("PublicKeyFile 未导入；PortMate 不会直接读取 PuTTY .ppk 私钥文件");
  }

  if (kind === "serial") {
    const port = normalizeSerialPort(raw.settings.get("serialline"));
    if (!port) {
      addCandidateWarning("SerialLine 为空或无效，已跳过");
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
    return candidate;
  }

  const host = normalizeHost(raw.settings.get("hostname"));
  if (!host) {
    addCandidateWarning("HostName 为空或无效，已跳过");
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
  if (kind === "ssh") applySshSettings(candidate, raw.settings, addCandidateWarning);
  return candidate;
}

function applySshSettings(
  candidate: PuttyNetworkImportCandidate,
  settings: Map<string, string>,
  addWarning: (message: string) => void,
) {
  const tryAgent = readBoolean(settings.get("tryagent"));
  if (settings.has("tryagent")) {
    if (tryAgent === null) addWarning("TryAgent 仅支持 0 或 1");
    else candidate.tryAgent = tryAgent;
  }
  const forwardAgent = readBoolean(settings.get("agentfwd"));
  if (settings.has("agentfwd")) {
    if (forwardAgent === null) addWarning("AgentFwd 仅支持 0 或 1");
    else candidate.forwardAgent = forwardAgent;
  }
}

function applyProxySettings(
  candidate: PuttyNetworkImportCandidate,
  settings: Map<string, string>,
  addWarning: (message: string) => void,
) {
  const password = settings.get("proxypassword")?.trim();
  if (password) addWarning("ProxyPassword 未导入；请在会话设置中重新录入代理密码");
  const rawMethod = settings.get("proxymethod");
  if (rawMethod === undefined) return;
  const method = parsePuttyInteger(rawMethod);
  if (method === null || method < 0) {
    addWarning("ProxyMethod 必须是有效整数，未导入代理");
    return;
  }
  if (method === 0) return;
  const kind = method === 2 ? "socks5" : method === 3 ? "http-connect" : null;
  if (!kind) {
    addWarning(`ProxyMethod=${method} 不受支持，未导入代理`);
    return;
  }
  const host = normalizeHost(settings.get("proxyhost"));
  const port = parsePort(settings.get("proxyport"));
  if (!host || port === null) {
    addWarning("ProxyHost 或 ProxyPort 无效，未导入代理");
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
    if (speed === null) addWarning("SerialSpeed 必须是 1 到 4000000 的整数");
    else candidate.serial.baudRate = speed;
  }
  const dataBits = parseIntegerInRange(settings.get("serialdatabits"), 5, 8);
  if (settings.has("serialdatabits")) {
    if (dataBits === null) addWarning("SerialDataBits 仅支持 5 到 8");
    else candidate.serial.dataBits = dataBits;
  }

  const stopHalfbits = parsePuttyInteger(settings.get("serialstophalfbits"));
  if (settings.has("serialstophalfbits")) {
    if (stopHalfbits === 2) candidate.serial.stopBits = 1;
    else if (stopHalfbits === 4) candidate.serial.stopBits = 2;
    else if (stopHalfbits === 3) addWarning("SerialStopHalfbits=3 表示 1.5 停止位，未导入");
    else addWarning("SerialStopHalfbits 仅支持 2 或 4");
  }

  const parity = parsePuttyInteger(settings.get("serialparity"));
  if (settings.has("serialparity")) {
    if (parity === 0) candidate.serial.parity = "none";
    else if (parity === 1) candidate.serial.parity = "odd";
    else if (parity === 2) candidate.serial.parity = "even";
    else addWarning("SerialParity 的 mark 或 space 模式未导入");
  }

  const flowControl = parsePuttyInteger(settings.get("serialflowcontrol"));
  if (settings.has("serialflowcontrol")) {
    if (flowControl === 0) candidate.serial.flowControl = "none";
    else if (flowControl === 1) candidate.serial.flowControl = "software";
    else if (flowControl === 2) candidate.serial.flowControl = "hardware";
    else addWarning("SerialFlowControl 的 DSR/DTR 模式未导入");
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
    addWarning("raw 会话缺少 PortNumber，已跳过");
    return null;
  }
  const port = parsePort(value);
  if (port === null) {
    addWarning("PortNumber 必须是 1 到 65535 的整数，已跳过");
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
    addWarning(`${label} 为空或无效，未导入`);
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
  return decodeSessionName(withoutExtension) || "PuTTY 会话";
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
