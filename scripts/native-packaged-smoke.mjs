import { createHash } from "node:crypto";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  statSync,
} from "node:fs";
import { isAbsolute, join, resolve } from "node:path";
import { spawn } from "node:child_process";

const pollIntervalMs = 100;
const maxOutputBytes = 128 * 1024;

export async function smokePackagedApplication({
  executable,
  args = [],
  dataDirectory,
  label,
  environment = process.env,
  exitAfterMs = 5_000,
  timeoutMs = 45_000,
  expectedStore = null,
}) {
  if (!executable || !isAbsolute(executable)) throw new Error(`${label} executable must be absolute`);
  if (!dataDirectory || !isAbsolute(dataDirectory)) throw new Error(`${label} data directory must be absolute`);
  if (!Number.isInteger(exitAfterMs) || exitAfterMs < 1_000 || exitAfterMs > 60_000) {
    throw new Error(`${label} exit delay must be between 1000 and 60000 ms`);
  }
  if (!Number.isInteger(timeoutMs) || timeoutMs <= exitAfterMs) {
    throw new Error(`${label} timeout must exceed its exit delay`);
  }
  mkdirSync(dataDirectory, { recursive: true });
  const endpointPath = join(dataDirectory, "portmate-ipc.json");
  const storePath = join(dataDirectory, "portmate-store.sqlite3");
  if (existsSync(endpointPath)) {
    throw new Error(`${label} smoke data directory contains a stale IPC endpoint`);
  }
  if (expectedStore) {
    assertStoreMatches(inspectStore(storePath, label), expectedStore, `${label} pre-launch Store`);
  } else if (existsSync(storePath)) {
    throw new Error(`${label} smoke data directory contains an unexpected Store`);
  }

  let output = "";
  let processResult = null;
  const child = spawn(executable, args, {
    cwd: dataDirectory,
    env: {
      ...environment,
      PORTMATE_NATIVE_SMOKE_DATA_DIR: dataDirectory,
      PORTMATE_NATIVE_SMOKE_EXIT_AFTER_MS: String(exitAfterMs),
    },
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  const exited = new Promise((resolveExit) => {
    const settle = (result) => {
      processResult ??= result;
      resolveExit(processResult);
    };
    child.once("error", (error) => settle({ error }));
    child.once("exit", (code, signal) => settle({ code, signal }));
  });
  for (const stream of [child.stdout, child.stderr]) {
    stream.on("data", (chunk) => {
      output = `${output}${chunk}`.slice(-maxOutputBytes);
    });
  }

  const deadline = Date.now() + timeoutMs;
  let endpoint;
  let smokeResult;
  let failure;
  try {
    endpoint = await waitForEndpoint(endpointPath, storePath, label, deadline, () => processResult);
    const result = await waitForExit(exited, deadline, label);
    if (result.error) throw result.error;
    if (result.code !== 0 || result.signal) {
      throw new Error(`${label} exited with ${result.code ?? result.signal}`);
    }
    await waitForRemoval(endpointPath, deadline, label);
    const store = inspectStore(storePath, label);
    if (expectedStore) assertStoreMatches(store, expectedStore, `${label} restarted Store`);
    smokeResult = {
      executable,
      endpointAddress: endpoint.addr,
      endpointUsesTokenRef: typeof endpoint.tokenRef === "string",
      endpointCredentialSha256: createHash("sha256")
        .update(endpoint.tokenRef ?? endpoint.token)
        .digest("hex"),
      gracefulExit: true,
      endpointRemoved: true,
      store,
    };
  } catch (error) {
    failure = error;
  } finally {
    try {
      await terminateChild(child, exited, () => processResult, label);
    } catch (error) {
      failure = failure
        ? new AggregateError([failure, error], `${label} runtime smoke and process cleanup both failed`)
        : error;
    }
  }
  if (failure) {
    const details = stripAnsi(output).trim().slice(-8_000);
    throw new Error(`${failure instanceof Error ? failure.message : String(failure)}${details ? `\n${details}` : ""}`, {
      cause: failure,
    });
  }
  return smokeResult;
}

export async function smokePackagedApplicationRestart(options) {
  const label = options?.label;
  const first = await smokePackagedApplication({
    ...options,
    label: `${label} initial launch`,
    expectedStore: null,
  });
  const second = await smokePackagedApplication({
    ...options,
    label: `${label} restart`,
    expectedStore: first.store,
  });
  if (first.endpointCredentialSha256 === second.endpointCredentialSha256) {
    throw new Error(`${label} reused its IPC credential after restart`);
  }
  return {
    first,
    second,
    storePreserved: true,
    endpointCredentialRotated: true,
  };
}

export function validatePackagedSmokeEndpoint(endpoint, storePath, label = "packaged app") {
  if (!endpoint || typeof endpoint !== "object" || Array.isArray(endpoint)) {
    throw new Error(`${label} IPC endpoint is not an object`);
  }
  if (!samePath(endpoint.storePath, storePath)) {
    throw new Error(`${label} IPC endpoint points outside its isolated Store`);
  }
  if (typeof endpoint.addr !== "string" || !/^127\.0\.0\.1:\d+$/.test(endpoint.addr)) {
    throw new Error(`${label} IPC endpoint is not loopback TCP`);
  }
  const hasTokenRef = typeof endpoint.tokenRef === "string" && endpoint.tokenRef.startsWith("keychain:ipc-");
  const hasFallbackToken = typeof endpoint.token === "string" && endpoint.token.length >= 16;
  if (hasTokenRef === hasFallbackToken) {
    throw new Error(`${label} IPC endpoint must contain exactly one token representation`);
  }
  return endpoint;
}

async function waitForEndpoint(endpointPath, storePath, label, deadline, processState) {
  let lastFailure = "endpoint was not published";
  while (Date.now() < deadline) {
    const exited = processState();
    if (exited) throw processExitError(exited, `${label} exited before publishing IPC`);
    if (existsSync(endpointPath)) {
      try {
        const endpoint = JSON.parse(readFileSync(endpointPath, "utf8"));
        return validatePackagedSmokeEndpoint(endpoint, storePath, label);
      } catch (error) {
        lastFailure = error instanceof Error ? error.message : String(error);
      }
    }
    await delay(pollIntervalMs);
  }
  throw new Error(`${label} did not publish a valid IPC endpoint: ${lastFailure}`);
}

async function waitForExit(exited, deadline, label) {
  const remaining = deadline - Date.now();
  if (remaining <= 0) throw new Error(`${label} exceeded its runtime smoke timeout`);
  return new Promise((resolveExit, reject) => {
    const timeout = setTimeout(() => {
      reject(new Error(`${label} did not exit after its native smoke deadline`));
    }, remaining);
    void exited.then((result) => {
      clearTimeout(timeout);
      resolveExit(result);
    });
  });
}

async function waitForRemoval(path, deadline, label) {
  while (Date.now() < deadline) {
    if (!existsSync(path)) return;
    await delay(pollIntervalMs);
  }
  throw new Error(`${label} left its IPC endpoint after normal exit`);
}

function inspectStore(path, label) {
  const metadata = statSync(path);
  if (!metadata.isFile() || metadata.size <= 0) throw new Error(`${label} did not persist a non-empty Store`);
  return {
    bytes: metadata.size,
    sha256: createHash("sha256").update(readFileSync(path)).digest("hex"),
  };
}

function assertStoreMatches(actual, expected, label) {
  if (!expected
      || !Number.isSafeInteger(expected.bytes)
      || expected.bytes <= 0
      || typeof expected.sha256 !== "string"
      || !/^[a-f0-9]{64}$/.test(expected.sha256)) {
    throw new Error(`${label} expectation is invalid`);
  }
  if (actual.bytes !== expected.bytes || actual.sha256 !== expected.sha256) {
    throw new Error(`${label} changed across an idle application restart`);
  }
}

function samePath(left, right) {
  if (typeof left !== "string" || !isAbsolute(left)) return false;
  const normalizedLeft = resolve(left);
  const normalizedRight = resolve(right);
  return process.platform === "win32"
    ? normalizedLeft.toLocaleLowerCase("en-US") === normalizedRight.toLocaleLowerCase("en-US")
    : normalizedLeft === normalizedRight;
}

function processExitError(result, prefix) {
  if (result.error) return new Error(`${prefix}: ${result.error.message}`, { cause: result.error });
  return new Error(`${prefix}: ${result.code ?? result.signal}`);
}

async function terminateChild(child, exited, processState, label) {
  if (processState() !== null) return;
  child.kill();
  if (await settlesWithin(exited, 5_000)) return;
  child.kill("SIGKILL");
  if (await settlesWithin(exited, 5_000)) return;
  throw new Error(`${label} process did not terminate after forced cleanup`);
}

async function settlesWithin(exited, timeoutMs) {
  let timeout;
  const settled = await Promise.race([
    exited.then(() => true),
    new Promise((resolveSettlement) => {
      timeout = setTimeout(() => resolveSettlement(false), timeoutMs);
    }),
  ]);
  clearTimeout(timeout);
  return settled;
}

function stripAnsi(value) {
  return value.replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, "");
}

function delay(milliseconds) {
  return new Promise((resolveDelay) => setTimeout(resolveDelay, milliseconds));
}
