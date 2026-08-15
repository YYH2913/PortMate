import { describe, expect, it } from "vitest";
import {
  createMcpGrant,
  formatMcpGrantExpiryInput,
  generateMcpClientId,
  mcpGrantDraftHasUnsavedChanges,
  parseMcpGrantExpiryInput,
} from "./mcp-grant-state";

describe("MCP grant editor state", () => {
  it("creates a bounded read-only-first grant draft", () => {
    expect(createMcpGrant()).toEqual({
      clientId: "",
      name: "",
      scopes: ["read-sessions", "read-logs", "read-transfers", "read-tunnels", "read-scripts"],
      allowedSessions: [],
      confirmWrites: true,
      expiresAt: null,
      revokedAt: null,
    });
  });

  it("detects grant edits while ignoring set ordering", () => {
    const saved = {
      ...createMcpGrant(),
      clientId: "ops-client",
      name: "Ops client",
      allowedSessions: ["session-a", "session-b"],
    };
    expect(mcpGrantDraftHasUnsavedChanges({
      ...saved,
      scopes: [...saved.scopes].reverse(),
      allowedSessions: [...saved.allowedSessions].reverse(),
    }, saved)).toBe(false);
    for (const changed of [
      { name: "Changed" },
      { confirmWrites: false },
      { expiresAt: "2031-04-05T06:07:00.000Z" },
      { scopes: ["read-sessions" as const] },
      { allowedSessions: ["session-a"] },
    ]) {
      expect(mcpGrantDraftHasUnsavedChanges({ ...saved, ...changed }, saved)).toBe(true);
    }
  });

  it("keeps a pristine new grant clean until the user enters a value", () => {
    expect(mcpGrantDraftHasUnsavedChanges(createMcpGrant(), null)).toBe(false);
    expect(mcpGrantDraftHasUnsavedChanges({ ...createMcpGrant(), clientId: "new-client" }, null)).toBe(true);
  });

  it("round-trips local minute inputs as canonical timestamps", () => {
    const input = "2031-04-05T06:07";
    const expiresAt = parseMcpGrantExpiryInput(input);
    expect(expiresAt).not.toBeNull();
    expect(formatMcpGrantExpiryInput(expiresAt)).toBe(input);
    expect(parseMcpGrantExpiryInput("")).toBeNull();
    expect(parseMcpGrantExpiryInput("invalid")).toBeNull();
    expect(formatMcpGrantExpiryInput("invalid")).toBe("");
  });

  it("rejects normalized overflow and incomplete expiry inputs", () => {
    expect(parseMcpGrantExpiryInput("2031-02-29T06:07")).toBeNull();
    expect(parseMcpGrantExpiryInput("2032-02-29T23:59")).not.toBeNull();
    expect(parseMcpGrantExpiryInput("2031-04-05")).toBeNull();
    expect(parseMcpGrantExpiryInput("2031-04-05T24:00")).toBeNull();
  });

  it("generates printable UUID-shaped client IDs from secure randomness", () => {
    const first = generateMcpClientId();
    const second = generateMcpClientId();
    expect(first).toMatch(/^client-[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    expect(second).not.toBe(first);
    expect(first.length).toBeLessThanOrEqual(128);
  });
});
