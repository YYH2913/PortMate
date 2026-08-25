import type { McpGrant } from "./types";

const MCP_CLIENT_ID_RANDOM_BYTES = 16;
// Kept as a reserved value so older stores that use [] for "all sessions"
// remain compatible while new grants can explicitly deny every session.
export const MCP_NO_SESSIONS_SENTINEL = "__portmate_no_sessions__";
export const DEFAULT_MCP_HTTP_CLIENT_ID = "portmate-local";
export type McpSessionAccessMode = "none" | "all" | "selected";

export function createMcpGrant(): McpGrant {
  return {
    clientId: "",
    name: "",
    scopes: ["read-sessions", "read-logs", "read-transfers", "read-tunnels", "read-scripts", "read-mcp"],
    allowedSessions: [MCP_NO_SESSIONS_SENTINEL],
    confirmWrites: true,
    expiresAt: null,
    revokedAt: null,
  };
}

export function mcpSessionAccessMode(grant: Pick<McpGrant, "allowedSessions">): McpSessionAccessMode {
  if (grant.allowedSessions.length === 1
    && grant.allowedSessions[0] === MCP_NO_SESSIONS_SENTINEL) return "none";
  return grant.allowedSessions.length ? "selected" : "all";
}

export function setMcpSessionAccessMode(
  grant: McpGrant,
  mode: McpSessionAccessMode,
): McpGrant {
  if (mode === "none") return { ...grant, allowedSessions: [MCP_NO_SESSIONS_SENTINEL] };
  if (mode === "all") return { ...grant, allowedSessions: [] };
  const selected = grant.allowedSessions.filter((id) => id !== MCP_NO_SESSIONS_SENTINEL);
  return { ...grant, allowedSessions: selected.length ? selected : [MCP_NO_SESSIONS_SENTINEL] };
}

export function resolveMcpHttpClientId(
  configured: string,
  grants: readonly Pick<McpGrant, "clientId" | "expiresAt" | "revokedAt">[],
): string {
  const configuredId = configured.trim();
  const active = grants.filter((grant) => (
    !grant.revokedAt
    && (!grant.expiresAt || Date.parse(grant.expiresAt) > Date.now())
  ));
  if (active.some((grant) => grant.clientId === configuredId)) return configuredId;
  const storedIsLegacyDefault = configuredId === "" || configuredId === DEFAULT_MCP_HTTP_CLIENT_ID;
  if (active.length === 1 && storedIsLegacyDefault) return active[0].clientId;
  if (configuredId && configuredId !== DEFAULT_MCP_HTTP_CLIENT_ID) return configuredId;
  return configuredId || DEFAULT_MCP_HTTP_CLIENT_ID;
}

export function mcpGrantDraftHasUnsavedChanges(
  draft: McpGrant | null,
  saved: McpGrant | null | undefined,
): boolean {
  if (!draft) return false;
  const baseline = saved ?? createMcpGrant();
  return draft.clientId !== baseline.clientId
    || draft.name !== baseline.name
    || draft.confirmWrites !== baseline.confirmWrites
    || (draft.expiresAt ?? null) !== (baseline.expiresAt ?? null)
    || (draft.revokedAt ?? null) !== (baseline.revokedAt ?? null)
    || !sameStringSet(draft.scopes, baseline.scopes)
    || !sameStringSet(draft.allowedSessions, baseline.allowedSessions);
}

export function generateMcpClientId(): string {
  const source = globalThis.crypto;
  if (!source?.getRandomValues) {
    throw new Error("当前环境不支持安全随机数，无法生成 Client ID");
  }
  const bytes = source.getRandomValues(new Uint8Array(MCP_CLIENT_ID_RANDOM_BYTES));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = [...bytes].map((value) => value.toString(16).padStart(2, "0"));
  return `client-${hex.slice(0, 4).join("")}-${hex.slice(4, 6).join("")}-${hex.slice(6, 8).join("")}-${hex.slice(8, 10).join("")}-${hex.slice(10).join("")}`;
}

export function formatMcpGrantExpiryInput(expiresAt: string | null | undefined): string {
  if (!expiresAt) return "";
  const date = new Date(expiresAt);
  if (Number.isNaN(date.getTime())) return "";
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60_000);
  return local.toISOString().slice(0, 16);
}

export function parseMcpGrantExpiryInput(value: string): string | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})T([01]\d|2[0-3]):([0-5]\d)$/.exec(value.trim());
  if (!match) return null;
  const [, yearText, monthText, dayText, hourText, minuteText] = match;
  const [year, month, day, hour, minute] = [yearText, monthText, dayText, hourText, minuteText].map(Number);
  const date = new Date(0);
  date.setFullYear(year, month - 1, day);
  date.setHours(hour, minute, 0, 0);
  if (date.getFullYear() !== year
    || date.getMonth() !== month - 1
    || date.getDate() !== day
    || date.getHours() !== hour
    || date.getMinutes() !== minute) return null;
  return date.toISOString();
}

function sameStringSet(left: readonly string[], right: readonly string[]): boolean {
  if (left.length !== right.length) return false;
  const expected = new Set(right);
  return expected.size === left.length && left.every((value) => expected.has(value));
}
