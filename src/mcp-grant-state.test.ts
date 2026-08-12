import { describe, expect, it } from "vitest";
import {
  createMcpGrant,
  formatMcpGrantExpiryInput,
  parseMcpGrantExpiryInput,
} from "./mcp-grant-state";

describe("MCP grant editor state", () => {
  it("creates a bounded read-only-first grant draft", () => {
    expect(createMcpGrant()).toEqual({
      clientId: "",
      name: "",
      scopes: ["read-sessions", "read-logs", "read-transfers", "read-tunnels"],
      allowedSessions: [],
      confirmWrites: true,
      expiresAt: null,
      revokedAt: null,
    });
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
});
