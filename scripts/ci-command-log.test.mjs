import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { afterEach, describe, expect, it } from "vitest";
import { parseLoggedCommandArguments } from "./ci-command-log.mjs";

const helper = resolve(import.meta.dirname, "ci-command-log.mjs");
const temporaryRoots = [];

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) {
    rmSync(root, { recursive: true, force: true });
  }
});

describe("CI command logging", () => {
  it("keeps a structured argv and rejects incomplete commands", () => {
    expect(parseLoggedCommandArguments(["diagnostics/build.log", "npm", "run", "build"]))
      .toMatchObject({ command: "npm", args: ["run", "build"] });
    expect(parseLoggedCommandArguments(["diagnostics/build.log", "npm"]).command).toBe("npm");
    expect(() => parseLoggedCommandArguments(["diagnostics/build.log"])).toThrow("Usage:");
    expect(() => parseLoggedCommandArguments(["bad\0path", "npm"])).toThrow("without NUL");
  });

  it("tees stdout and stderr into a path containing spaces and preserves failure status", () => {
    const root = mkdtempSync(join(tmpdir(), "portmate ci command log "));
    temporaryRoots.push(root);
    const logPath = join(root, "nested reports", "package audit.log");
    const result = spawnSync(process.execPath, [
      helper,
      logPath,
      process.execPath,
      "-e",
      "process.stdout.write('audit-out\\n' + 'x'.repeat(262144)); process.stderr.write('audit-error\\n'); process.exitCode = 7",
    ], { encoding: "utf8" });

    expect(result.status).toBe(7);
    expect(result.stdout).toContain("audit-out");
    expect(result.stderr).toContain("audit-error");
    const log = readFileSync(logPath, "utf8");
    expect(log).toContain("audit-out");
    expect(log).toContain("audit-error");
    expect(log.match(/x/g)).toHaveLength(262144);
  });

  it("retains both native credential fault gates as CI diagnostics", () => {
    const workflow = readFileSync(
      resolve(import.meta.dirname, "..", ".github", "workflows", "native-ci.yml"),
      "utf8",
    );
    expect(workflow).toContain(
      "target/native-ci/portable-vault.log npm run test:portable-vault",
    );
    expect(workflow).toContain(
      "target/native-ci/native-keyring.log npm run test:native-keyring",
    );
  });
});
