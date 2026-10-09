import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

/** Isolated read-only grants for official SDK compatibility checks, never user data. */
export function createMcpClientFixture(clientIds) {
  if (!Array.isArray(clientIds) || !clientIds.length
    || clientIds.some((id) => typeof id !== "string" || !/^[a-z0-9-]{1,128}$/.test(id))
    || new Set(clientIds).size !== clientIds.length) {
    throw new Error("MCP compatibility fixture needs unique explicit client IDs");
  }
  const root = mkdtempSync(join(tmpdir(), "portmate-sdk-read-fixture-"));
  const storePath = join(root, "test-store.json");
  let disposed = false;
  const dispose = () => {
    if (disposed) return;
    disposed = true;
    process.removeListener("exit", dispose);
    rmSync(root, { recursive: true, force: true, maxRetries: 3, retryDelay: 100 });
  };
  const setReadAccess = (allowed) => {
    if (disposed) throw new Error("MCP compatibility fixture was disposed");
    const store = {
      profiles: [], runtimes: [], events: [], transfers: [], hostKeys: { keys: [] },
      grants: allowed ? clientIds.map((clientId) => ({
        clientId, name: "Official SDK read-only test", scopes: ["read-sessions"],
        allowedSessions: [], confirmWrites: true, expiresAt: null, revokedAt: null,
      })) : [],
      audit: [], timeline: [], sysmon: [],
    };
    writeFileSync(storePath, JSON.stringify(store), { encoding: "utf8", mode: 0o600 });
  };
  try {
    setReadAccess(true);
    process.once("exit", dispose);
    return {
      storePath, setReadAccess, dispose,
      environment: {
        PORTMATE_MCP_TEST_STORE_PATH: storePath,
        PORTMATE_STORE_PATH: storePath,
        PORTMATE_MCP_TRUSTED: "0",
      },
    };
  } catch (error) {
    dispose();
    throw error;
  }
}
