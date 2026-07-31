import {
  mkdirSync,
  mkdtempSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { fileURLToPath } from "node:url";

const scriptPath = fileURLToPath(import.meta.url);
const projectRoot = resolve(dirname(scriptPath), "..");
const accountEnvironment = "PORTMATE_NATIVE_KEYRING_PROBE_ACCOUNT";
const secretEnvironment = "PORTMATE_NATIVE_KEYRING_PROBE_SECRET";
const rotatedSecretEnvironment = "PORTMATE_NATIVE_KEYRING_PROBE_ROTATED_SECRET";
const probeSecretBytes = 1_200;

try {
  if (process.platform === "linux" && process.argv[2] !== "--linux-session") {
    runIsolatedLinuxSecretService();
  } else {
    if (process.platform === "linux") {
      verifyUnavailableLinuxSecretService();
      startLinuxSecretService();
    }
    if (process.platform === "darwin") {
      verifyIsolatedMacOSKeychain();
    } else {
      runProbe(process.env);
      if (process.platform === "linux") verifyLockedLinuxSecretService();
      if (process.platform === "win32") verifyDeniedWindowsCredentialManager();
    }
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
    DISPLAY: "",
    WAYLAND_DISPLAY: "",
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

function verifyUnavailableLinuxSecretService() {
  runProbePhase("expect-unavailable", {
    ...process.env,
    DBUS_SESSION_BUS_ADDRESS: `unix:path=${join(process.env.XDG_RUNTIME_DIR, "missing-secret-service.sock")}`,
  });
}

function verifyLockedLinuxSecretService() {
  const environment = {
    ...process.env,
    [accountEnvironment]: `native-probe-locked-${randomUUID()}`,
    [secretEnvironment]: probeSecret("locked"),
    [rotatedSecretEnvironment]: probeSecret("locked-rotated"),
  };
  runProbePhase("write", environment);
  run("gdbus", [
    "call",
    "--session",
    "--dest",
    "org.freedesktop.secrets",
    "--object-path",
    "/org/freedesktop/secrets",
    "--method",
    "org.freedesktop.Secret.Service.Lock",
    "[objectpath '/org/freedesktop/secrets/aliases/default']",
  ], { stdio: "pipe" });
  runProbePhase("verify-locked", environment);
  console.log("PortMate native keyring fault probe passed on linux (unavailable and locked provider)");
}

function verifyDeniedWindowsCredentialManager() {
  const environment = {
    ...process.env,
    [accountEnvironment]: `native-probe-windows-denied-${randomUUID()}`,
    [secretEnvironment]: probeSecret("windows-denied"),
    [rotatedSecretEnvironment]: probeSecret("windows-denied-rotated"),
  };
  runProbePhase("verify-denied", environment);
  console.log("PortMate native keyring fault probe passed on windows (anonymous token denied provider)");
}

function verifyIsolatedMacOSKeychain() {
  const root = mkdtempSync(join(tmpdir(), "portmate-native-keychain-"));
  const keychain = join(root, "PortMateProbe.keychain-db");
  const password = `portmate-${randomUUID()}`;
  const originalDefault = capture("security", ["default-keychain"])
    .split(/\r?\n/)
    .map((line) => line.trim())
    .find((line) => line.startsWith('"') && line.endsWith('"'))
    ?.slice(1, -1);
  if (!originalDefault) {
    rmSync(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
    throw new Error("无法读取 macOS default keychain，拒绝修改系统钥匙串设置");
  }
  const environment = {
    ...process.env,
    [accountEnvironment]: `native-probe-macos-${randomUUID()}`,
    [secretEnvironment]: probeSecret("macos"),
    [rotatedSecretEnvironment]: probeSecret("macos-rotated"),
  };
  let keychainCreated = false;
  let defaultReplaced = false;
  try {
    run("security", ["create-keychain", "-p", password, keychain], { stdio: "pipe" });
    keychainCreated = true;
    run("security", ["set-keychain-settings", "-lut", "3600", keychain], { stdio: "pipe" });
    run("security", ["unlock-keychain", "-p", password, keychain], { stdio: "pipe" });
    run("security", ["default-keychain", "-s", keychain], { stdio: "pipe" });
    defaultReplaced = true;
    runProbePhase("write", environment);
    runProbePhase("verify-update", environment);
    run("security", ["lock-keychain", keychain], { stdio: "pipe" });
    runProbePhase("verify-locked", environment);
    run("security", ["unlock-keychain", "-p", password, keychain], { stdio: "pipe" });
    runProbePhase("verify-delete", environment);
    console.log("PortMate native keyring fault probe passed on macos (isolated default keychain and locked provider)");
  } finally {
    if (keychainCreated) {
      try {
        run("security", ["unlock-keychain", "-p", password, keychain], { stdio: "pipe" });
      } catch {
        // The restore below is still required if the temporary keychain stayed locked.
      }
      try {
        runProbePhase("cleanup", environment);
      } catch {
        // The primary probe error is more useful than best-effort cleanup output.
      }
      if (defaultReplaced) {
        run("security", ["default-keychain", "-s", originalDefault], { stdio: "pipe" });
      }
      run("security", ["delete-keychain", keychain], { stdio: "pipe" });
      rmSync(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
    } else {
      rmSync(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
    }
  }
}

function runProbe(environment) {
  runCargoProbe([], environment);
}

function runProbePhase(phase, environment) {
  runCargoProbe(["--phase", phase], environment);
}

function runCargoProbe(probeArguments, environment) {
  run("cargo", [
    "run",
    "--locked",
    "-p",
    "portmate",
    "--bin",
    "native-keyring-probe",
    "--",
    ...probeArguments,
  ], { env: environment, timeout: 360_000 });
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    env: options.env ?? process.env,
    encoding: "utf8",
    stdio: options.stdio ?? "inherit",
    timeout: options.timeout ?? 30_000,
  });
  if (result.error || result.status !== 0) throw commandError(command, result);
}

function capture(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    env: options.env ?? process.env,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    timeout: options.timeout ?? 30_000,
  });
  if (result.error || result.status !== 0) throw commandError(command, result);
  return result.stdout;
}

function probeSecret(label) {
  return `portmate-native-keyring-${label}-${randomUUID()}`.padEnd(
    probeSecretBytes,
    "x",
  );
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
