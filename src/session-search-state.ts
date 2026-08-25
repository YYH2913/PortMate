import type { SessionEvent, SessionProfile, SessionSummary } from "./types";

export type SessionSearchMode = "sessions" | "logs";

export type SessionSearchResult = {
  key: string;
  sessionId: string;
  title: string;
  detail: string;
};

const MAX_LOG_SEARCH_RESULTS = 80;
const MAX_LOG_RESULT_CHARACTERS = 2_048;

export function filterWorkspaceSessions(
  sessions: readonly SessionSummary[],
  query: string,
): SessionSummary[] {
  // Hydration and profile-update events can briefly contain the same profile
  // twice. Keep the explorer/search contract one row per session ID while
  // retaining the newest matching summary.
  const needle = normalizeSearchText(query);
  const candidates = needle
    ? sessions.filter((session) => sessionSearchText(session).includes(needle))
    : [...sessions];
  const unique = new Map<string, SessionSummary>();
  for (const session of candidates) unique.set(session.profile.id, session);
  return [...unique.values()];
}

export function buildSessionSearchResults(
  mode: SessionSearchMode,
  query: string,
  sessions: readonly SessionSummary[],
  logs: Readonly<Record<string, readonly SessionEvent[]>>,
): SessionSearchResult[] {
  if (mode === "sessions") {
    return filterWorkspaceSessions(sessions, query).map((session) => ({
      key: `session-${session.profile.id}`,
      sessionId: session.profile.id,
      title: session.profile.name,
      detail: sessionResultDetail(session),
    }));
  }

  const needle = normalizeSearchText(query);
  const sessionsById = new Map(sessions.map((session) => [session.profile.id, session]));
  return Object.values(logs)
    .flat()
    .filter((event) => !needle || logSearchText(event, sessionsById.get(event.sessionId)).includes(needle))
    .sort((left, right) => eventTimestamp(right.ts) - eventTimestamp(left.ts))
    .slice(0, MAX_LOG_SEARCH_RESULTS)
    .map((event) => {
      const session = sessionsById.get(event.sessionId);
      const commandId = event.annotations.commandId;
      const commandLabel = commandId ? `[命令 ${commandId.slice(0, 8)}] ` : "";
      return {
        key: `log-${event.sessionId}-${event.id}`,
        sessionId: event.sessionId,
        title: session?.profile.name ?? event.sessionId,
        detail: boundedText(`${event.direction} · ${commandLabel}${event.text ?? ""}`.trim()),
      };
    });
}

export function describeSessionEndpoint(profile: SessionProfile): string {
  const connection = profile.connection;
  switch (connection.kind) {
    case "ssh":
    case "tmux":
      return connection.username
        ? `${connection.username}@${connection.endpoint.host}:${connection.endpoint.port}`
        : `${connection.endpoint.host}:${connection.endpoint.port}`;
    case "serial":
      return connection.port || "serial";
    case "shell":
      return connection.program || "shell";
    case "telnet":
    case "tcp":
      return `${connection.host}:${connection.port}`;
  }
}

function sessionResultDetail(session: SessionSummary): string {
  const context = [
    session.profile.kind.toUpperCase(),
    session.runtime.status,
    describeSessionEndpoint(session.profile),
    session.profile.group,
    ...session.profile.tags,
  ].filter(Boolean);
  return context.join(" · ");
}

function sessionSearchText(session: SessionSummary): string {
  const connection = session.profile.connection;
  let endpoint: string;
  switch (connection.kind) {
    case "ssh":
    case "tmux":
      endpoint = `${connection.username} ${connection.endpoint.host} ${connection.endpoint.port}`;
      break;
    case "tcp":
    case "telnet":
      endpoint = `${connection.host} ${connection.port}`;
      break;
    case "serial":
      endpoint = `${connection.port} ${connection.baudRate}`;
      break;
    case "shell":
      endpoint = `${connection.program} ${connection.args.join(" ")} ${connection.cwd ?? ""}`;
      break;
  }
  return normalizeSearchText([
    session.profile.id,
    session.profile.name,
    session.profile.group,
    session.profile.kind,
    session.runtime.status,
    ...session.profile.tags,
    endpoint,
  ].join(" "));
}

function logSearchText(event: SessionEvent, session?: SessionSummary): string {
  return normalizeSearchText([
    event.text ?? "",
    event.annotations.commandId ?? "",
    event.direction,
    event.stream,
    event.sessionId,
    session?.profile.name ?? "",
    session?.profile.group ?? "",
    ...(session?.profile.tags ?? []),
  ].join(" "));
}

function normalizeSearchText(value: string): string {
  return value.trim().toLowerCase().replace(/\s+/g, " ");
}

function eventTimestamp(value: string): number {
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

function boundedText(value: string): string {
  const characters = Array.from(value);
  if (characters.length <= MAX_LOG_RESULT_CHARACTERS) return value;
  return `${characters.slice(0, MAX_LOG_RESULT_CHARACTERS).join("")}...`;
}
