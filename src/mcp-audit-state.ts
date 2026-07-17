import type { AuditRecord, McpScope } from "./types";

export const MCP_AUDIT_GLOBAL_SESSION = "__global__";

export type McpAuditFilters = {
  query: string;
  decision: string;
  sessionId: string;
  scope: "" | McpScope;
};

export function filterMcpAudit(records: AuditRecord[], filters: McpAuditFilters): AuditRecord[] {
  const query = filters.query.trim().toLocaleLowerCase();
  return records
    .filter((record) => {
      if (filters.decision && record.decision !== filters.decision) return false;
      if (filters.sessionId === MCP_AUDIT_GLOBAL_SESSION && record.sessionId) return false;
      if (filters.sessionId && filters.sessionId !== MCP_AUDIT_GLOBAL_SESSION && record.sessionId !== filters.sessionId) return false;
      if (filters.scope && record.details.scope !== filters.scope) return false;
      if (!query) return true;
      const searchable = [
        record.id,
        record.ts,
        record.actor,
        record.action,
        record.sessionId ?? "",
        record.decision,
        ...Object.entries(record.details).flat(),
      ].join(" ").toLocaleLowerCase();
      return searchable.includes(query);
    })
    .sort((left, right) => {
      const timestamp = Date.parse(right.ts) - Date.parse(left.ts);
      return Number.isNaN(timestamp) || timestamp === 0 ? left.id.localeCompare(right.id) : timestamp;
    });
}

export function mcpAuditDecisionOptions(records: AuditRecord[]): string[] {
  return [...new Set(records.map((record) => record.decision).filter(Boolean))].sort();
}
