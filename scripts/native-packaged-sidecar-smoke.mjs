import { randomBytes } from "node:crypto";
import {
  existsSync,
  lstatSync,
  mkdtempSync,
  readFileSync,
  rmSync,
} from "node:fs";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { temporaryDirectoryFromEnvironment } from "./native-package-workspace.mjs";

const helper = fileURLToPath(new URL("./native-packaged-sidecar-parent.mjs", import.meta.url));
const maximumCapturedOutputBytes = 64 * 1024;
const maximumReportedDiagnosticBytes = 8 * 1024;

export async function smokePackagedSidecarParentWatchdog({
  executable,
  args = ["--http"],
  label,
  environment = process.env,
  readyTimeoutMs = 8_000,
  watchdogTimeoutMs = 8_000,
  token = randomBytes(32).toString("hex"),
}) {
  validateOptions({ executable, args, label, readyTimeoutMs, watchdogTimeoutMs, token });
  const resolvedExecutable = resolve(executable);
  const smokeRoot = mkdtempSync(join(
    temporaryDirectoryFromEnvironment(environment),
    "portmate packaged sidecar watchdog ",
  ));
  const reportPath = join(smokeRoot, "parent report.json");
  let parentOutput = Buffer.alloc(0);
  let parentResult = null;
  let report = null;
  let failure;
  let result;

  const parent = spawn(process.execPath, [helper], {
    cwd: smokeRoot,
    env: {
      ...environment,
      PORTMATE_PACKAGE_SMOKE_ARGS: JSON.stringify(args),
      PORTMATE_PACKAGE_SMOKE_CWD: dirname(resolvedExecutable),
      PORTMATE_PACKAGE_SMOKE_EXECUTABLE: resolvedExecutable,
      PORTMATE_PACKAGE_SMOKE_READY_TIMEOUT_MS: String(readyTimeoutMs),
      PORTMATE_PACKAGE_SMOKE_REPORT: reportPath,
      PORTMATE_PACKAGE_SMOKE_TOKEN: token,
    },
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  const parentExited = new Promise((resolveExit) => {
    const settle = (processResult) => {
      parentResult ??= processResult;
      resolveExit(parentResult);
    };
    parent.once("error", (error) => settle({ error }));
    parent.once("exit", (code, signal) => settle({ code, signal }));
  });
  for (const stream of [parent.stdout, parent.stderr]) {
    stream.on("data", (chunk) => {
      const captureLimit = maximumCapturedOutputBytes + Buffer.byteLength(token);
      parentOutput = Buffer.concat([parentOutput, chunk]).subarray(-captureLimit);
    });
  }

  try {
    const processResult = await waitForExit(
      parentExited,
      readyTimeoutMs + 5_000,
      `${label} watchdog parent`,
    );
    report = readReport(reportPath, label);
    if (processResult.error) throw processResult.error;
    if (processResult.code !== 0 || processResult.signal) {
      throw new Error(
        `${label} watchdog parent failed with ${processResult.code ?? processResult.signal}: ${report.error ?? "no diagnostic"}`,
      );
    }
    validateReadyReport(report, label);
    if (!await waitUntilProcessExits(report.pid, watchdogTimeoutMs)) {
      throw new Error(`${label} sidecar ${report.pid} survived its parent beyond ${watchdogTimeoutMs} ms`);
    }
    result = {
      executable: resolvedExecutable,
      endpoint: report.endpoint,
      protocolVersion: report.protocolVersion,
      sidecarPid: report.pid,
      readyProbe: true,
      parentExited: true,
      sidecarExited: true,
    };
  } catch (error) {
    failure = error;
  } finally {
    try {
      await terminateChild(parent, parentExited, () => parentResult, `${label} watchdog parent`);
    } catch (error) {
      failure = combineFailures(failure, error, `${label} parent cleanup failed`);
    }
    if (!report && existsSync(reportPath)) {
      try {
        report = readReport(reportPath, label);
      } catch (error) {
        failure = combineFailures(failure, error, `${label} report parsing failed`);
      }
    }
    if (!result && isProcessId(report?.pid)) {
      try {
        await terminateProcessId(report.pid, `${label} sidecar`);
      } catch (error) {
        failure = combineFailures(failure, error, `${label} sidecar cleanup failed`);
      }
    }
    try {
      rmSync(smokeRoot, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
    } catch (error) {
      failure = combineFailures(failure, error, `${label} smoke-directory cleanup failed`);
    }
  }

  if (failure) {
    const diagnostics = boundedDiagnostic(
      [report?.diagnostic, parentOutput.toString("utf8")]
        .filter((value) => typeof value === "string" && value.trim())
        .join("\n"),
      token,
      maximumReportedDiagnosticBytes,
    );
    const message = redactToken(failureMessage(failure), token);
    throw new Error(`${message}${diagnostics ? `\n${diagnostics}` : ""}`, { cause: failure });
  }
  return result;
}

function validateOptions({ executable, args, label, readyTimeoutMs, watchdogTimeoutMs, token }) {
  if (typeof executable !== "string" || !isAbsolute(executable)) {
    throw new Error(`${label ?? "packaged sidecar"} executable must be absolute`);
  }
  const metadata = lstatSync(executable);
  if (!metadata.isFile()) throw new Error(`${label} executable must be a regular file`);
  if (!Array.isArray(args) || args.length > 64 || args.some((arg) => typeof arg !== "string")) {
    throw new Error(`${label} arguments must be an array of at most 64 strings`);
  }
  if (typeof label !== "string" || !label.trim()) throw new Error("packaged sidecar label is required");
  validateTimeout(readyTimeoutMs, `${label} readiness timeout`);
  validateTimeout(watchdogTimeoutMs, `${label} watchdog timeout`);
  if (typeof token !== "string" || token.length < 16 || token.length > 4_096 || token.includes("\0")) {
    throw new Error(`${label} smoke token must contain 16 to 4096 characters and no NUL`);
  }
}

function validateTimeout(value, label) {
  if (!Number.isInteger(value) || value < 250 || value > 60_000) {
    throw new Error(`${label} must be between 250 and 60000 ms`);
  }
}

function readReport(path, label) {
  if (!existsSync(path)) throw new Error(`${label} watchdog parent did not publish a report`);
  let report;
  try {
    report = JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    throw new Error(`${label} watchdog parent published invalid JSON: ${failureMessage(error)}`);
  }
  if (!report || typeof report !== "object" || Array.isArray(report)) {
    throw new Error(`${label} watchdog parent report is not an object`);
  }
  return report;
}

function validateReadyReport(report, label) {
  if (report.phase !== "ready" || report.ready !== true) {
    throw new Error(`${label} watchdog parent did not confirm sidecar readiness`);
  }
  if (!isProcessId(report.pid) || report.pid === process.pid) {
    throw new Error(`${label} watchdog parent published an invalid sidecar PID`);
  }
  if (typeof report.endpoint !== "string" || !/^http:\/\/127\.0\.0\.1:\d+\/mcp$/.test(report.endpoint)) {
    throw new Error(`${label} watchdog parent published an invalid loopback endpoint`);
  }
  if (typeof report.protocolVersion !== "string" || !report.protocolVersion.trim()) {
    throw new Error(`${label} readiness response omitted MCP-Protocol-Version`);
  }
}

async function waitForExit(exited, timeoutMs, label) {
  let timeout;
  const result = await Promise.race([
    exited,
    new Promise((_, reject) => {
      timeout = setTimeout(() => reject(new Error(`${label} exceeded ${timeoutMs} ms`)), timeoutMs);
    }),
  ]);
  clearTimeout(timeout);
  return result;
}

async function terminateChild(child, exited, processState, label) {
  if (processState() !== null) return;
  child.kill();
  if (await settlesWithin(exited, 2_000)) return;
  child.kill("SIGKILL");
  if (await settlesWithin(exited, 3_000)) return;
  throw new Error(`${label} resisted forced cleanup`);
}

async function terminateProcessId(pid, label) {
  if (!processExists(pid)) return;
  signalProcess(pid, "SIGTERM");
  if (await waitUntilProcessExits(pid, 2_000)) return;
  signalProcess(pid, "SIGKILL");
  if (await waitUntilProcessExits(pid, 3_000)) return;
  throw new Error(`${label} ${pid} resisted forced cleanup`);
}

function signalProcess(pid, signal) {
  try {
    process.kill(pid, signal);
  } catch (error) {
    if (error?.code !== "ESRCH") throw error;
  }
}

async function waitUntilProcessExits(pid, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (!processExists(pid)) return true;
    await delay(25);
  }
  return !processExists(pid);
}

function processExists(pid) {
  if (!isProcessId(pid)) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    if (error?.code === "ESRCH") return false;
    if (error?.code === "EPERM") return true;
    throw error;
  }
}

async function settlesWithin(promise, timeoutMs) {
  let timeout;
  const settled = await Promise.race([
    promise.then(() => true),
    new Promise((resolveSettlement) => {
      timeout = setTimeout(() => resolveSettlement(false), timeoutMs);
    }),
  ]);
  clearTimeout(timeout);
  return settled;
}

function isProcessId(value) {
  return Number.isInteger(value) && value > 0 && value <= 0xffff_ffff;
}

function combineFailures(current, next, message) {
  return current ? new AggregateError([current, next], message) : next;
}

function failureMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

function redactToken(value, token) {
  return value.split(token).join("[REDACTED]");
}

function boundedDiagnostic(value, token, maximumBytes) {
  return Buffer.from(redactToken(value, token).trim(), "utf8")
    .subarray(-maximumBytes)
    .toString("utf8");
}

function delay(milliseconds) {
  return new Promise((resolveDelay) => setTimeout(resolveDelay, milliseconds));
}
