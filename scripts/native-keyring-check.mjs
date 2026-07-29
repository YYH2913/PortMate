import {
  mkdirSync,
  mkdtempSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const scriptPath = fileURLToPath(import.meta.url);
const projectRoot = resolve(dirname(scriptPath), "..");

try {
  if (process.platform === "linux" && process.argv[2] !== "--linux-session") {
    runIsolatedLinuxSecretService();
  } else {
    if (process.platform === "linux") startLinuxSecretService();
    runProbe(process.env);
  }
} catch (error) {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
}

function runIsolatedLinuxSecretService() {
  const originalHome = process.env.HOME;
  if (!originalHome && (!process.env.CARGO_HOME || !process.env.RUSTUP_HOME)) {
    throw new Error(
      "HOME is required when CARGO_HOME or RUSTUP_HOME is not configured",
    );
  }
  const root = mkdtempSync(join(tmpdir(), "portmate-native-keyring-"));
  const environment = {
    ...process.env,
    CARGO_HOME: process.env.CARGO_HOME ?? join(originalHome, ".cargo"),
    HOME: join(root, "home"),
    RUSTUP_HOME: process.env.RUSTUP_HOME ?? join(originalHome, ".rustup"),
    XDG_CACHE_HOME: join(root, "cache"),
    XDG_CONFIG_HOME: join(root, "config"),
    XDG_DATA_HOME: join(root, "data"),
    XDG_RUNTIME_DIR: join(root, "runtime"),
  };
  for (const path of [
    environment.HOME,
    environment.XDG_CACHE_HOME,
    environment.XDG_CONFIG_HOME,
    environment.XDG_DATA_HOME,
    environment.XDG_RUNTIME_DIR,
  ]) {
    mkdirSync(path, { recursive: true, mode: 0o700 });
  }
  for (const name of ["DBUS_SESSION_BUS_ADDRESS", "GNOME_KEYRING_CONTROL", "SSH_AUTH_SOCK"]) {
    delete environment[name];
  }
  try {
    run("dbus-run-session", ["--", process.execPath, scriptPath, "--linux-session"], {
      env: environment,
      timeout: 360_000,
    });
  } finally {
    rmSync(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  }
}

function startLinuxSecretService() {
  const result = spawnSync(
    "gnome-keyring-daemon",
    ["--unlock", "--components=secrets"],
    {
      cwd: projectRoot,
      env: process.env,
      input: "portmate-native-keyring-probe\n",
      encoding: "utf8",
      stdio: ["pipe", "pipe", "pipe"],
      timeout: 30_000,
    },
  );
  if (result.error) throw commandError("gnome-keyring-daemon", result);
  if (result.status !== 0) throw commandError("gnome-keyring-daemon", result);
  for (const line of result.stdout.split(/\r?\n/)) {
    const match = /^(GNOME_KEYRING_CONTROL|SSH_AUTH_SOCK)=([^\0\r\n]+)$/.exec(line.trim());
    if (match) process.env[match[1]] = match[2];
  }
}

function runProbe(environment) {
  run("cargo", [
    "run",
    "--locked",
    "-p",
    "portmate",
    "--bin",
    "native-keyring-probe",
  ], { env: environment, timeout: 360_000 });
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    env: options.env ?? process.env,
    encoding: "utf8",
    stdio: "inherit",
    timeout: options.timeout ?? 30_000,
  });
  if (result.error || result.status !== 0) throw commandError(command, result);
}

function commandError(command, result) {
  if (result.error?.code === "ETIMEDOUT") {
    return new Error(`${command} exceeded its configured timeout`);
  }
  const details = [result.stdout, result.stderr]
    .filter(Boolean)
    .join("\n")
    .trim()
    .slice(-4_096);
  return new Error(
    `${command} failed with exit code ${result.status ?? 1}${details ? `\n${details}` : ""}`,
    { cause: result.error },
  );
}
