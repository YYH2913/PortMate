import { spawn } from "node:child_process";
import { writeFileSync } from "node:fs";
import { createServer, request } from "node:http";
import { isAbsolute } from "node:path";

const diagnosticLimit = 8 * 1024;
const pollIntervalMs = 50;
const probeTimeoutMs = 500;
const helperEnvironmentKeys = [
  "PORTMATE_PACKAGE_SMOKE_ARGS",
  "PORTMATE_PACKAGE_SMOKE_CWD",
  "PORTMATE_PACKAGE_SMOKE_EXECUTABLE",
  "PORTMATE_PACKAGE_SMOKE_READY_TIMEOUT_MS",
  "PORTMATE_PACKAGE_SMOKE_REPORT",
  "PORTMATE_PACKAGE_SMOKE_TOKEN",
];

const executable = requiredAbsolutePath("PORTMATE_PACKAGE_SMOKE_EXECUTABLE");
const cwd = requiredAbsolutePath("PORTMATE_PACKAGE_SMOKE_CWD");
const reportPath = requiredAbsolutePath("PORTMATE_PACKAGE_SMOKE_REPORT");
const token = requiredEnvironmentValue("PORTMATE_PACKAGE_SMOKE_TOKEN");
if (token.length > 4_096) throw new Error("PORTMATE_PACKAGE_SMOKE_TOKEN is too long");
const diagnosticCaptureLimit = diagnosticLimit + Buffer.byteLength(token);
const readyTimeoutMs = boundedIntegerEnvironment(
  "PORTMATE_PACKAGE_SMOKE_READY_TIMEOUT_MS",
  250,
  60_000,
);
const args = parseArguments(requiredEnvironmentValue("PORTMATE_PACKAGE_SMOKE_ARGS"));

let child;
let childResult = null;
let output = Buffer.alloc(0);
let endpoint;

try {
  const port = await reservePort();
  endpoint = `http://127.0.0.1:${port}/mcp`;
  const childEnvironment = { ...process.env };
  for (const key of helperEnvironmentKeys) delete childEnvironment[key];
  Object.assign(childEnvironment, {
    PORTMATE_MCP_CLIENT_ID: "packaged-parent-watchdog-smoke",
    PORTMATE_MCP_HTTP: "1",
    PORTMATE_MCP_HTTP_ADDR: `127.0.0.1:${port}`,
    PORTMATE_MCP_HTTP_ALLOW_REMOTE: "0",
    PORTMATE_MCP_HTTP_ORIGINS: "",
    PORTMATE_MCP_HTTP_TOKEN: token,
    PORTMATE_MCP_PARENT_PID: String(process.pid),
    PORTMATE_MCP_TRUSTED: "0",
    PORTMATE_STORE_PATH: "",
  });
  child = spawn(executable, args, {
    cwd,
    env: childEnvironment,
    stdio: ["ignore", "ignore", "pipe"],
    windowsHide: true,
  });
  const exited = new Promise((resolveExit) => {
    const settle = (result) => {
      childResult ??= result;
      resolveExit(childResult);
    };
    child.once("error", (error) => settle({ error }));
    child.once("close", (code, signal) => settle({ code, signal }));
  });
  child.stderr.on("data", (chunk) => {
    output = Buffer.concat([output, chunk]).subarray(-diagnosticCaptureLimit);
  });
  await waitForSpawn(child, exited);
  writeReport({ phase: "starting", pid: child.pid, endpoint });

  const protocolVersion = await waitForReadiness(endpoint, readyTimeoutMs, () => childResult);
  writeReport({
    phase: "ready",
    pid: child.pid,
    endpoint,
    protocolVersion,
    ready: true,
  });
  child.unref();
  process.exit(0);
} catch (error) {
  let failure = error;
  if (child) {
    try {
      await terminateChild(child, () => childResult);
    } catch (cleanupError) {
      failure = new AggregateError(
        [failure, cleanupError],
        "packaged sidecar readiness and cleanup both failed",
      );
    }
  }
  const message = redactToken(failureMessage(failure));
  const diagnostic = boundedDiagnostic(output.toString("utf8"));
  writeReport({
    phase: "failed",
    pid: child?.pid ?? null,
    endpoint: endpoint ?? null,
    error: message,
    diagnostic,
  });
  process.stderr.write(`${message}\n`);
  process.exit(1);
}

async function waitForReadiness(url, timeoutMs, processState) {
  const deadline = Date.now() + timeoutMs;
  let lastFailure = "listener did not accept a connection";
  while (Date.now() < deadline) {
    const exited = processState();
    if (exited) throw processExitError(exited, "sidecar exited before HTTP readiness");
    try {
      const response = await probe(url);
      if (response.statusCode === 204 && response.protocolVersion) {
        return response.protocolVersion;
      }
      lastFailure = response.statusCode === 204
        ? "OPTIONS /mcp omitted MCP-Protocol-Version"
        : `OPTIONS /mcp returned HTTP ${response.statusCode}`;
    } catch (error) {
      lastFailure = failureMessage(error);
    }
    await delay(pollIntervalMs);
  }
  throw new Error(`sidecar did not become ready: ${lastFailure}`);
}

function probe(url) {
  return new Promise((resolveProbe, reject) => {
    const probeRequest = request(url, { method: "OPTIONS" }, (response) => {
      response.resume();
      response.once("end", () => resolveProbe({
        statusCode: response.statusCode,
        protocolVersion: headerValue(response.headers["mcp-protocol-version"]),
      }));
    });
    probeRequest.setTimeout(probeTimeoutMs, () => {
      probeRequest.destroy(new Error("OPTIONS /mcp timed out"));
    });
    probeRequest.once("error", reject);
    probeRequest.end();
  });
}

async function reservePort() {
  const server = createServer();
  await new Promise((resolveListen, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolveListen);
  });
  const address = server.address();
  const port = typeof address === "object" && address ? address.port : 0;
  await new Promise((resolveClose, reject) => {
    server.close((error) => error ? reject(error) : resolveClose());
  });
  if (!Number.isInteger(port) || port <= 0) throw new Error("failed to reserve a loopback port");
  return port;
}

function waitForSpawn(processChild, exited) {
  if (Number.isInteger(processChild.pid) && processChild.pid > 0) return Promise.resolve();
  return Promise.race([
    new Promise((resolveSpawn, reject) => {
      processChild.once("spawn", resolveSpawn);
      processChild.once("error", reject);
    }),
    exited.then((result) => {
      throw processExitError(result, "sidecar failed to start");
    }),
  ]);
}

async function terminateChild(processChild, processState) {
  if (processState() !== null) return;
  processChild.kill();
  if (await waitUntil(() => processState() !== null, 2_000)) return;
  processChild.kill("SIGKILL");
  if (await waitUntil(() => processState() !== null, 3_000)) return;
  throw new Error(`sidecar process ${processChild.pid ?? "unknown"} resisted forced cleanup`);
}

function processExitError(result, prefix) {
  if (result.error) return new Error(`${prefix}: ${result.error.message}`, { cause: result.error });
  return new Error(`${prefix}: ${result.code ?? result.signal}`);
}

function writeReport(report) {
  writeFileSync(reportPath, `${JSON.stringify(report)}\n`, { mode: 0o600 });
}

function requiredEnvironmentValue(name) {
  const value = process.env[name];
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`${name} is required`);
  }
  return value;
}

function requiredAbsolutePath(name) {
  const value = requiredEnvironmentValue(name);
  if (!isAbsolute(value)) throw new Error(`${name} must be absolute`);
  return value;
}

function boundedIntegerEnvironment(name, minimum, maximum) {
  const value = Number.parseInt(requiredEnvironmentValue(name), 10);
  if (!Number.isInteger(value) || value < minimum || value > maximum) {
    throw new Error(`${name} must be between ${minimum} and ${maximum}`);
  }
  return value;
}

function parseArguments(value) {
  let parsed;
  try {
    parsed = JSON.parse(value);
  } catch (error) {
    throw new Error(`PORTMATE_PACKAGE_SMOKE_ARGS is not valid JSON: ${failureMessage(error)}`);
  }
  if (!Array.isArray(parsed) || parsed.length > 64 || parsed.some((arg) => typeof arg !== "string")) {
    throw new Error("PORTMATE_PACKAGE_SMOKE_ARGS must be an array of at most 64 strings");
  }
  return parsed;
}

function headerValue(value) {
  if (Array.isArray(value)) return value.find((entry) => entry.trim())?.trim() ?? "";
  return typeof value === "string" ? value.trim() : "";
}

function failureMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

function redactToken(value) {
  return value.split(token).join("[REDACTED]");
}

function boundedDiagnostic(value) {
  return Buffer.from(redactToken(value).trim(), "utf8")
    .subarray(-diagnosticLimit)
    .toString("utf8");
}

async function waitUntil(predicate, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (predicate()) return true;
    await delay(25);
  }
  return predicate();
}

function delay(milliseconds) {
  return new Promise((resolveDelay) => setTimeout(resolveDelay, milliseconds));
}
