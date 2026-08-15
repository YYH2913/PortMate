import { existsSync, readFileSync, rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const bootstrap = findPython();
const matrix = JSON.parse(readFileSync(join(projectRoot, "scripts", "mcp-python-client-versions.json"), "utf8"));
if (!Array.isArray(matrix) || !matrix.length || matrix.some((entry) => (
  typeof entry !== "object"
  || !/^\d+\.\d+\.\d+$/.test(entry.version)
  || !/^\d{4}-\d{2}-\d{2}$/.test(entry.protocolVersion)
))) {
  throw new Error("scripts/mcp-python-client-versions.json must contain exact SDK and protocol versions");
}

for (const { version: sdkVersion, protocolVersion } of matrix) {
  const environmentRoot = join(projectRoot, "target", `mcp-python-sdk-${sdkVersion}`);
  const lockPath = join(
    projectRoot,
    "scripts",
    "mcp-python-client-check",
    "locks",
    sdkVersion,
    "requirements.txt",
  );
  if (!existsSync(lockPath)) {
    throw new Error(`MCP Python SDK ${sdkVersion} lock file does not exist: ${lockPath}`);
  }
  const expectedSnapshot = normalizedSnapshot(readFileSync(lockPath, "utf8"));
  if (!expectedSnapshot.split("\n").includes(`mcp==${sdkVersion}`)) {
    throw new Error(`MCP Python SDK ${sdkVersion} lock file pins a different SDK version`);
  }
  const environmentPython = process.platform === "win32"
    ? join(environmentRoot, "Scripts", "python.exe")
    : join(environmentRoot, "bin", "python");
  if (!existsSync(environmentPython)) {
    run(bootstrap.command, [...bootstrap.args, "-m", "venv", environmentRoot]);
  }

  let installedSnapshot = pythonPackageSnapshot(environmentPython);
  if (installedSnapshot !== expectedSnapshot) {
    rmSync(environmentRoot, { recursive: true, force: true, maxRetries: 3, retryDelay: 100 });
    run(bootstrap.command, [...bootstrap.args, "-m", "venv", environmentRoot]);
    run(environmentPython, [
      "-m",
      "pip",
      "install",
      "--disable-pip-version-check",
      "--requirement",
      lockPath,
    ], { timeout: 300_000 });
    installedSnapshot = pythonPackageSnapshot(environmentPython);
  }
  if (installedSnapshot !== expectedSnapshot) {
    throw new Error(`MCP Python SDK ${sdkVersion} environment does not match its dependency lock`);
  }

  run(environmentPython, [join(projectRoot, "scripts", "mcp-python-client-check.py")], {
    env: {
      ...process.env,
      PORTMATE_MCP_PYTHON_SDK_VERSION: sdkVersion,
      PORTMATE_MCP_EXPECTED_PROTOCOL_VERSION: protocolVersion,
    },
  });
}

function findPython() {
  const configured = process.env.PORTMATE_PYTHON?.trim();
  const candidates = configured
    ? [{ command: configured, args: [] }]
    : process.platform === "win32"
      ? [{ command: "py", args: ["-3"] }, { command: "python", args: [] }]
      : [{ command: "python3", args: [] }, { command: "python", args: [] }];
  for (const candidate of candidates) {
    const probe = spawnSync(candidate.command, [...candidate.args, "-c", "import sys; raise SystemExit(sys.version_info < (3, 10))"], {
      cwd: projectRoot,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      timeout: 10_000,
    });
    if (!probe.error && probe.status === 0) return candidate;
  }
  throw new Error("MCP Python SDK checks require Python 3.10 or newer (set PORTMATE_PYTHON to override discovery)");
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    env: options.env ?? process.env,
    encoding: "utf8",
    stdio: options.capture ? ["ignore", "pipe", "pipe"] : "inherit",
    maxBuffer: 16 * 1024 * 1024,
    timeout: options.timeout ?? 300_000,
  });
  if (result.error) throw result.error;
  if (result.status !== 0 && !options.allowFailure) {
    const details = [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
    throw new Error(`${command} failed with exit code ${result.status ?? 1}${details ? `\n${details}` : ""}`);
  }
  return result;
}

function pythonPackageSnapshot(environmentPython) {
  const result = run(
    environmentPython,
    ["-m", "pip", "freeze", "--exclude", "pip"],
    { capture: true, allowFailure: true, timeout: 30_000 },
  );
  return result.status === 0 ? normalizedSnapshot(result.stdout) : "";
}

function normalizedSnapshot(value) {
  return value.trim().replace(/\r\n?/g, "\n");
}
