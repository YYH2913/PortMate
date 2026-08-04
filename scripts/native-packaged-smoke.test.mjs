import {
  existsSync,
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
  smokePackagedApplicationRestart,
  smokePackagedApplicationRestartAndLegacyMigration,
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

  it("preserves the Store and rotates the IPC credential across restart", async () => {
    const root = temporaryRoot();
    const fixture = join(root, "restart-fixture.mjs");
    writeFileSync(fixture, `
      import { randomBytes } from "node:crypto";
      import { existsSync, rmSync, writeFileSync } from "node:fs";
      import { join } from "node:path";
      const data = process.env.PORTMATE_NATIVE_SMOKE_DATA_DIR;
      const store = join(data, "portmate-store.sqlite3");
      const endpoint = join(data, "portmate-ipc.json");
      if (!existsSync(store)) writeFileSync(store, "stable-sqlite-fixture");
      writeFileSync(endpoint, JSON.stringify({
        addr: "127.0.0.1:43123",
        storePath: store,
        token: randomBytes(24).toString("hex"),
      }));
      setTimeout(() => rmSync(endpoint), 100);
      setTimeout(() => process.exit(0), 150);
    `);

    const result = await smokePackagedApplicationRestart({
      executable: process.execPath,
      args: [fixture],
      dataDirectory: join(root, "restart-data", "dev.portmate.desktop"),
      label: "restart fixture app",
      exitAfterMs: 1_000,
      timeoutMs: 5_000,
    });

    expect(result.storePreserved).toBe(true);
    expect(result.endpointCredentialRotated).toBe(true);
    expect(result.first.store).toEqual(result.second.store);
    expect(result.first.endpointCredentialSha256).not.toBe(result.second.endpointCredentialSha256);
  });

  it("preserves the Store while migrating the legacy application data directory", async () => {
    const root = temporaryRoot();
    const fixture = join(root, "migration-fixture.mjs");
    writeFileSync(fixture, `
      import { randomBytes } from "node:crypto";
      import { existsSync, readdirSync, renameSync, rmSync, writeFileSync } from "node:fs";
      import { dirname, join } from "node:path";
      const data = process.env.PORTMATE_NATIVE_SMOKE_DATA_DIR;
      const legacy = join(dirname(data), "dev.portmate.app");
      if (existsSync(legacy)) {
        if (!existsSync(data) || readdirSync(data).length !== 0) process.exit(91);
        rmSync(data, { recursive: true });
        renameSync(legacy, data);
      }
      const store = join(data, "portmate-store.sqlite3");
      const endpoint = join(data, "portmate-ipc.json");
      if (!existsSync(store)) writeFileSync(store, randomBytes(48));
      writeFileSync(endpoint, JSON.stringify({
        addr: "127.0.0.1:43123",
        storePath: store,
        token: randomBytes(24).toString("hex"),
      }));
      setTimeout(() => rmSync(endpoint), 100);
      setTimeout(() => process.exit(0), 150);
    `);
    const dataDirectory = join(root, "migration-data", "dev.portmate.desktop");

    const result = await smokePackagedApplicationRestartAndLegacyMigration({
      executable: process.execPath,
      args: [fixture],
      dataDirectory,
      label: "migration fixture app",
      exitAfterMs: 1_000,
      timeoutMs: 5_000,
    });

    expect(result.legacyAppDataMigrated).toBe(true);
    expect(result.endpointCredentialRotatedAfterMigration).toBe(true);
    expect(result.migration.store).toEqual(result.second.store);
    expect(result.migration.endpointCredentialSha256).not.toBe(result.second.endpointCredentialSha256);
    expect(existsSync(join(root, "migration-data", "dev.portmate.app"))).toBe(false);
  });

  it("rejects an application that ignores the staged legacy data directory", async () => {
    const { fixture, dataDirectory } = restartFailureFixture("ignore-legacy-directory");
    await expect(smokePackagedApplicationRestartAndLegacyMigration({
      executable: process.execPath,
      args: [fixture, "normal"],
      dataDirectory,
      label: "ignored migration fixture app",
      exitAfterMs: 1_000,
      timeoutMs: 5_000,
    })).rejects.toThrow(/left the legacy app-data directory/);
  });

  it("rejects an idle restart that mutates the Store", async () => {
    const { fixture, dataDirectory } = restartFailureFixture("mutate-store");
    await expect(smokePackagedApplicationRestart({
      executable: process.execPath,
      args: [fixture, "mutate-store"],
      dataDirectory,
      label: "Store mutation fixture app",
      exitAfterMs: 1_000,
      timeoutMs: 5_000,
    })).rejects.toThrow(/changed across an idle application restart/);
  });

  it("rejects an idle restart that reuses its IPC credential", async () => {
    const { fixture, dataDirectory } = restartFailureFixture("reuse-credential");
    await expect(smokePackagedApplicationRestart({
      executable: process.execPath,
      args: [fixture, "reuse-credential"],
      dataDirectory,
      label: "credential reuse fixture app",
      exitAfterMs: 1_000,
      timeoutMs: 5_000,
    })).rejects.toThrow(/reused its IPC credential after restart/);
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

function restartFailureFixture(name) {
  const root = temporaryRoot();
  const fixture = join(root, `${name}.mjs`);
  writeFileSync(fixture, `
    import { randomBytes } from "node:crypto";
    import { appendFileSync, existsSync, rmSync, writeFileSync } from "node:fs";
    import { join } from "node:path";
    const data = process.env.PORTMATE_NATIVE_SMOKE_DATA_DIR;
    const behavior = process.argv[2];
    const store = join(data, "portmate-store.sqlite3");
    const endpoint = join(data, "portmate-ipc.json");
    if (!existsSync(store)) writeFileSync(store, "stable-sqlite-fixture");
    else if (behavior === "mutate-store") appendFileSync(store, "-changed");
    const token = behavior === "reuse-credential"
      ? "fixture-static-token-with-enough-entropy"
      : randomBytes(24).toString("hex");
    writeFileSync(endpoint, JSON.stringify({ addr: "127.0.0.1:43123", storePath: store, token }));
    setTimeout(() => rmSync(endpoint), 100);
    setTimeout(() => process.exit(0), 150);
  `);
  return {
    fixture,
    dataDirectory: join(root, "data", "dev.portmate.desktop"),
  };
}
