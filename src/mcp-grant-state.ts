import type { McpGrant } from "./types";

const MCP_CLIENT_ID_RANDOM_BYTES = 16;

export function createMcpGrant(): McpGrant {
  return {
    clientId: "",
    name: "",
    scopes: ["read-sessions", "read-logs", "read-transfers", "read-tunnels"],
    allowedSessions: [],
    confirmWrites: true,
    expiresAt: null,
    revokedAt: null,
  };
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
