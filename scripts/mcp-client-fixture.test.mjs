import { existsSync, readFileSync, statSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { createMcpClientFixture } from "./mcp-client-fixture.mjs";

describe("MCP SDK read-only fixture", () => {
  it("isolates grants, supports revocation and cleans up idempotently", () => {
    const first = createMcpClientFixture(["sdk-stdio", "sdk-http"]);
    const second = createMcpClientFixture(["other-sdk"]);
    try {
      expect(first.storePath).not.toBe(second.storePath);
      const read = (fixture) => JSON.parse(readFileSync(fixture.storePath, "utf8"));
      expect(read(first).grants.map((grant) => grant.clientId)).toEqual(["sdk-stdio", "sdk-http"]);
      expect(read(first).grants.every((grant) => grant.scopes.length === 1 && grant.scopes[0] === "read-sessions" && grant.confirmWrites)).toBe(true);
      expect(read(first).profiles).toEqual([]);
      expect(first.environment.PORTMATE_STORE_PATH).toBe(first.storePath);
      expect(first.environment.PORTMATE_MCP_TEST_STORE_PATH).toBe(first.storePath);
      expect(first.environment.PORTMATE_MCP_TRUSTED).toBe("0");
      if (process.platform !== "win32") expect(statSync(first.storePath).mode & 0o777).toBe(0o600);
      first.setReadAccess(false);
      expect(read(first).grants).toEqual([]);
      expect(read(second).grants).toHaveLength(1);
      first.setReadAccess(true);
      expect(read(first).grants).toHaveLength(2);
    } finally {
      first.dispose(); second.dispose();
    }
    first.dispose();
    expect(existsSync(first.storePath)).toBe(false);
    expect(() => first.setReadAccess(true)).toThrow("disposed");
  });

  it("rejects missing, ambiguous or invalid test identities", () => {
    for (const ids of [[], [""], ["a", "a"], ["bad\nclient"], [null]]) {
      expect(() => createMcpClientFixture(ids)).toThrow("unique explicit client IDs");
    }
  });
});
