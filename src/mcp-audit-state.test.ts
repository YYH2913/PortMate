import { describe, expect, it } from "vitest";
import { filterMcpAudit, MCP_AUDIT_GLOBAL_SESSION, mcpAuditDecisionOptions } from "./mcp-audit-state";
import type { AuditRecord } from "./types";

const records: AuditRecord[] = [
  { id: "old", ts: "2026-07-17T01:00:00Z", actor: "mcp:alpha", action: "send_text", sessionId: "edge", decision: "succeeded", details: { scope: "write-input", bootstrap: "false" } },
  { id: "new", ts: "2026-07-17T02:00:00Z", actor: "mcp:beta", action: "list_sessions", sessionId: null, decision: "authorized", details: { scope: "read-sessions" } },
  { id: "middle", ts: "2026-07-17T01:30:00Z", actor: "mcp:alpha", action: "create_tunnel", sessionId: "lab", decision: "denied", details: { scope: "tunnel", reason: "grant missing" } },
];

describe("MCP audit filtering", () => {
  it("sorts newest first and searches record context and details", () => {
    expect(filterMcpAudit(records, { query: "", decision: "", sessionId: "", scope: "" }).map((record) => record.id))
      .toEqual(["new", "middle", "old"]);
    expect(filterMcpAudit(records, { query: "grant missing", decision: "", sessionId: "", scope: "" }).map((record) => record.id))
      .toEqual(["middle"]);
    expect(filterMcpAudit(records, { query: "BETA", decision: "", sessionId: "", scope: "" }).map((record) => record.id))
      .toEqual(["new"]);
  });

  it("combines decision, session and scope filters", () => {
    expect(filterMcpAudit(records, { query: "", decision: "succeeded", sessionId: "edge", scope: "write-input" }).map((record) => record.id))
      .toEqual(["old"]);
    expect(filterMcpAudit(records, { query: "", decision: "", sessionId: MCP_AUDIT_GLOBAL_SESSION, scope: "read-sessions" }).map((record) => record.id))
      .toEqual(["new"]);
  });

  it("derives stable decision options", () => {
    expect(mcpAuditDecisionOptions([...records, records[0]])).toEqual(["authorized", "denied", "succeeded"]);
  });
});
