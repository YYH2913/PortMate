import {
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { smokePackagedSidecarParentWatchdog } from "./native-packaged-sidecar-smoke.mjs";

const temporaryRoots = [];

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) {
    rmSync(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 });
  }
});

describe("native packaged sidecar parent watchdog smoke", () => {
  it("confirms HTTP readiness and observes sidecar exit after a parent path containing spaces exits", async () => {
    const { fixture, pidPath } = sidecarFixture("successful watchdog");

    const result = await smokePackagedSidecarParentWatchdog({
      executable: process.execPath,
      args: [fixture, "watchdog", pidPath],
      label: "watchdog fixture",
      readyTimeoutMs: 2_000,
      watchdogTimeoutMs: 2_000,
    });

    expect(result.readyProbe).toBe(true);
    expect(result.parentExited).toBe(true);
    expect(result.sidecarExited).toBe(true);
    expect(result.protocolVersion).toBe("2025-06-18");
    expect(result.sidecarPid).toBe(Number.parseInt(readFileSync(pidPath, "utf8"), 10));
    expect(processExists(result.sidecarPid)).toBe(false);
  });

  it("reports a sidecar that exits before readiness", async () => {
    const { fixture, pidPath } = sidecarFixture("early exit");

    await expect(smokePackagedSidecarParentWatchdog({
      executable: process.execPath,
      args: [fixture, "early-exit", pidPath],
      label: "early-exit fixture",
      readyTimeoutMs: 1_000,
      watchdogTimeoutMs: 1_000,
    })).rejects.toThrow(/sidecar exited before HTTP readiness: 17/);

    expect(processExists(pidFrom(pidPath))).toBe(false);
  });

  it("rejects an invalid readiness response and cleans up the sidecar", async () => {
    const { fixture, pidPath } = sidecarFixture("invalid readiness");

    await expect(smokePackagedSidecarParentWatchdog({
      executable: process.execPath,
      args: [fixture, "invalid-readiness", pidPath],
      label: "invalid-readiness fixture",
      readyTimeoutMs: 500,
      watchdogTimeoutMs: 1_000,
    })).rejects.toThrow(/sidecar did not become ready: OPTIONS \/mcp returned HTTP 200/);

    expect(processExists(pidFrom(pidPath))).toBe(false);
  });

  it("times out a sidecar that ignores parent exit and forcibly cleans it up", async () => {
    const { fixture, pidPath } = sidecarFixture("ignored parent");

    await expect(smokePackagedSidecarParentWatchdog({
      executable: process.execPath,
      args: [fixture, "ignore-parent", pidPath],
      label: "ignored-parent fixture",
      readyTimeoutMs: 1_000,
      watchdogTimeoutMs: 300,
    })).rejects.toThrow(/sidecar \d+ survived its parent beyond 300 ms/);

    expect(processExists(pidFrom(pidPath))).toBe(false);
  });

  it("bounds diagnostics and redacts the HTTP token", async () => {
    const { fixture, pidPath } = sidecarFixture("bounded diagnostics");
    const token = "fixture-http-token-must-never-escape";
    let failure;

    try {
      await smokePackagedSidecarParentWatchdog({
        executable: process.execPath,
        args: [fixture, "noisy-invalid-readiness", pidPath],
        label: "noisy fixture",
        readyTimeoutMs: 500,
        watchdogTimeoutMs: 1_000,
        token,
      });
    } catch (error) {
      failure = error;
    }

    expect(failure).toBeInstanceOf(Error);
    expect(failure.message).toContain("[REDACTED]");
    expect(failure.message).not.toContain(token);
    expect(Buffer.byteLength(failure.message)).toBeLessThan(10 * 1024);
    expect(processExists(pidFrom(pidPath))).toBe(false);
  });
});

function sidecarFixture(name) {
  const root = mkdtempSync(join(tmpdir(), `portmate sidecar fixture ${name} `));
  temporaryRoots.push(root);
  const fixture = join(root, "sidecar fixture.mjs");
  const pidPath = join(root, "sidecar process.pid");
  writeFileSync(fixture, `
    import { writeFileSync } from "node:fs";
    import { createServer } from "node:http";

    const behavior = process.argv[2];
    const pidPath = process.argv[3];
    const parentPid = Number.parseInt(process.env.PORTMATE_MCP_PARENT_PID, 10);
    const port = Number.parseInt(process.env.PORTMATE_MCP_HTTP_ADDR.split(":").at(-1), 10);
    writeFileSync(pidPath, String(process.pid));

    if (behavior === "early-exit") {
      process.stderr.write("fixture exited before readiness\\n");
      process.exit(17);
    }
    if (behavior === "noisy-invalid-readiness") {
      process.stderr.write("x".repeat(32 * 1024));
      process.stderr.write(\` token=\${process.env.PORTMATE_MCP_HTTP_TOKEN}\\n\`);
    }

    const server = createServer((request, response) => {
      if (request.method !== "OPTIONS" || request.url !== "/mcp") {
        response.writeHead(404).end();
        return;
      }
      if (behavior === "invalid-readiness" || behavior === "noisy-invalid-readiness") {
        response.writeHead(200).end();
        return;
      }
      response.writeHead(204, { "MCP-Protocol-Version": "2025-06-18" }).end();
    });
    server.listen(port, "127.0.0.1");

    if (behavior === "watchdog") {
      setInterval(() => {
        try {
          process.kill(parentPid, 0);
        } catch {
          server.close(() => process.exit(0));
        }
      }, 25);
    } else {
      setInterval(() => {}, 60_000);
    }
  `);
  return { fixture, pidPath };
}

function pidFrom(path) {
  return Number.parseInt(readFileSync(path, "utf8"), 10);
}

function processExists(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    if (error?.code === "EPERM") return true;
    return false;
  }
}
