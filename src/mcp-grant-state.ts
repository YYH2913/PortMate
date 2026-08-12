import type { McpGrant } from "./types";

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

export function formatMcpGrantExpiryInput(expiresAt: string | null | undefined): string {
  if (!expiresAt) return "";
  const date = new Date(expiresAt);
  if (Number.isNaN(date.getTime())) return "";
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60_000);
  return local.toISOString().slice(0, 16);
}

export function parseMcpGrantExpiryInput(value: string): string | null {
  if (!value.trim()) return null;
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? null : date.toISOString();
}
