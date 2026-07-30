import {
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  smokePackagedApplication,
  validatePackagedSmokeEndpoint,
} from "./native-packaged-smoke.mjs";

const temporaryRoots = [];

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) {
    rmSync(root, { recursive: true, force: true });
  }
});

describe("native packaged runtime smoke", () => {
  it("observes endpoint publication, Store creation, and normal cleanup", async () => {
    const root = temporaryRoot();
    const fixture = join(root, "fixture.mjs");
    writeFileSync(fixture, `
      import { mkdirSync, rmSync, writeFileSync } from "node:fs";
      import { join } from "node:path";
      const data = process.env.PORTMATE_NATIVE_SMOKE_DATA_DIR;
      mkdirSync(data, { recursive: true });
      const store = join(data, "portmate-store.sqlite3");
      const endpoint = join(data, "portmate-ipc.json");
      writeFileSync(store, "sqlite-fixture");
      writeFileSync(endpoint, JSON.stringify({
        addr: "127.0.0.1:43123",
        storePath: store,
        token: "fixture-token-with-enough-entropy",
      }));
      setTimeout(() => rmSync(endpoint), 250);
      setTimeout(() => process.exit(0), 350);
    `);
    const dataDirectory = join(root, "data", "dev.portmate.desktop");

    const result = await smokePackagedApplication({
      executable: process.execPath,
      args: [fixture],
      dataDirectory,
      label: "fixture app",
      exitAfterMs: 1_000,
      timeoutMs: 5_000,
    });

    expect(result.gracefulExit).toBe(true);
    expect(result.endpointRemoved).toBe(true);
    expect(result.endpointUsesTokenRef).toBe(false);
    expect(result.store.bytes).toBeGreaterThan(0);
  });

  it("rejects foreign stores and ambiguous credentials", () => {
    const root = temporaryRoot();
    const store = join(root, "portmate-store.sqlite3");
    expect(() => validatePackagedSmokeEndpoint({
      addr: "127.0.0.1:43123",
      storePath: join(root, "other.sqlite3"),
      token: "fixture-token-with-enough-entropy",
    }, store)).toThrow(/outside its isolated Store/);
    expect(() => validatePackagedSmokeEndpoint({
      addr: "127.0.0.1:43123",
      storePath: store,
      tokenRef: "keychain:ipc-fixture",
      token: "fixture-token-with-enough-entropy",
    }, store)).toThrow(/exactly one token representation/);
  });

  it("terminates a hanging packaged process before reporting failure", async () => {
    const root = temporaryRoot();
    const fixture = join(root, "hanging-fixture.mjs");
    const pidFile = join(root, "fixture.pid");
    writeFileSync(fixture, `
      import { writeFileSync } from "node:fs";
      import { join } from "node:path";
      const data = process.env.PORTMATE_NATIVE_SMOKE_DATA_DIR;
      writeFileSync(process.argv[2], String(process.pid));
      writeFileSync(join(data, "portmate-ipc.json"), "not-json");
      setInterval(() => {}, 60_000);
    `);

    await expect(smokePackagedApplication({
      executable: process.execPath,
      args: [fixture, pidFile],
      dataDirectory: join(root, "data", "dev.portmate.desktop"),
      label: "hanging fixture app",
      exitAfterMs: 1_000,
      timeoutMs: 1_200,
    })).rejects.toThrow(/did not publish a valid IPC endpoint/);

    const pid = Number.parseInt(readFileSync(pidFile, "utf8"), 10);
    expect(() => process.kill(pid, 0)).toThrow();
  });
});

function temporaryRoot() {
  const root = mkdtempSync(join(tmpdir(), "portmate-native-packaged-smoke-"));
  temporaryRoots.push(root);
  return root;
}
