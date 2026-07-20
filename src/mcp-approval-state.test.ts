import { describe, expect, it } from "vitest";
import { mergeMcpApprovals, normalizeMcpApproval } from "./mcp-approval-state";
import type { McpApprovalRequest } from "./types";

const base: McpApprovalRequest = {
  id: "123e4567-e89b-42d3-a456-426614174000",
  clientId: "ops-client",
  action: "run_command",
  sessionId: "edge-router",
  scope: "write-input",
  createdAt: "2026-07-17T10:00:00.000Z",
  expiresAt: "2026-07-17T10:01:00.000Z",
};

describe("MCP approval state", () => {
  it("accepts bounded action/scope pairs and canonicalizes timestamps", () => {
    expect(normalizeMcpApproval(base)).toEqual(base);
    expect(normalizeMcpApproval({ ...base, action: "create_tunnel", scope: "write-input" })).toBeNull();
    expect(normalizeMcpApproval({ ...base, clientId: "bad\nclient" })).toBeNull();
    expect(normalizeMcpApproval({ ...base, expiresAt: "2026-07-17T10:02:00.000Z" })).toBeNull();
  });

  it("deduplicates, sorts and removes expired approvals", () => {
    const older = { ...base, id: "123e4567-e89b-42d3-a456-426614174001", createdAt: "2026-07-17T09:59:30.000Z", expiresAt: "2026-07-17T10:00:30.000Z" };
    const expired = { ...base, id: "123e4567-e89b-42d3-a456-426614174002", createdAt: "2026-07-17T09:58:00.000Z", expiresAt: "2026-07-17T09:59:00.000Z" };
    expect(mergeMcpApprovals([base], [base, older, expired], Date.parse("2026-07-17T10:00:00.000Z")).map((item) => item.id))
      .toEqual([older.id, base.id]);
    expect(mergeMcpApprovals([], [base, older], Date.parse("2026-07-17T10:00:00.000Z"), new Set([base.id])))
      .toEqual([older]);
  });

  it("rejects malformed IDs, missing fields and unknown actions", () => {
    expect(normalizeMcpApproval({ ...base, id: "not-a-uuid" })).toBeNull();
    expect(normalizeMcpApproval({ ...base, action: "delete_everything" })).toBeNull();
    expect(normalizeMcpApproval({ ...base, sessionId: "" })).toBeNull();
  });

  it("treats malformed pending-list responses as empty", () => {
    const now = Date.parse("2026-07-17T10:00:00.000Z");
    expect(mergeMcpApprovals([base], null, now)).toEqual([base]);
    expect(mergeMcpApprovals([], { approvals: [base] }, now)).toEqual([]);
    expect(mergeMcpApprovals([], "not-a-list", now)).toEqual([]);
  });
});
