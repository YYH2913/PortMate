import { spawn, spawnSync } from "node:child_process";
import { readFileSync, readlinkSync } from "node:fs";
import { createServer } from "node:net";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";
import { buildDesktopEnvironment } from "./desktop-clean-environment.mjs";
import {
  isProjectViteCommand,
  parseWindowsListeningPids,
  releaseProjectDevPort,
  signalProcessIfRunning,
  waitForStablePortAvailability,
} from "./desktop-clean-process.mjs";
import { assertSupportedNodeVersion } from "./ensure-node-version.mjs";

const PORT_RELEASE_POLL_MS = 100;
const PORT_RELEASE_STABLE_MS = 750;
// The total probe budget must exceed the stability window; otherwise normal
// socket-probe overhead can make an already-free port look busy.
const PORT_RELEASE_INITIAL_WAIT_MS = PORT_RELEASE_STABLE_MS * 2;

assertSupportedNodeVersion();

const projectRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));
const tauriConfig = JSON.parse(readFileSync(resolve(projectRoot, "src-tauri/tauri.conf.json"), "utf8"));
const devUrl = new URL(tauriConfig.build.devUrl);
const devHost = devUrl.hostname;
const devPort = Number(devUrl.port || (devUrl.protocol === "https:" ? 443 : 80));

const env = buildDesktopEnvironment(process.env);

await releaseConfiguredDevPort();

const child = spawn("npm", ["run", "desktop"], {
  stdio: "inherit",
  env,
  cwd: projectRoot,
});

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
  }
  process.exit(code ?? 0);
});

async function releaseConfiguredDevPort() {
  await releaseProjectDevPort({
    host: devHost,
    port: devPort,
    initialWaitMs: PORT_RELEASE_INITIAL_WAIT_MS,
    terminateWaitMs: 2_000,
    forceWaitMs: 1_000,
    waitForPort,
    listeningPids,
    isProjectVite,
    signalProcess: signalProcessIfRunning,
    onStopped: console.log,
  });
}

function listeningPids(port) {
  if (process.platform === "linux") {
    const result = spawnSync("fuser", ["-n", "tcp", String(port)], { encoding: "utf8" });
    return parsePids(result.stdout ?? "");
  }
  if (process.platform === "darwin") {
    const result = spawnSync("lsof", ["-nP", `-iTCP:${port}`, "-sTCP:LISTEN", "-t"], { encoding: "utf8" });
    return parsePids(result.stdout ?? "");
  }
  if (process.platform === "win32") {
    const result = spawnSync("netstat", ["-ano", "-p", "tcp"], {
      encoding: "utf8",
      windowsHide: true,
    });
    return parseWindowsListeningPids(result.stdout ?? "", port);
  }
  return [];
}

function parsePids(value) {
  return [...new Set((value.match(/\b\d+\b/g) ?? []).map(Number).filter((pid) => pid > 1))];
}

function isProjectVite(pid) {
  if (process.platform === "win32") {
    const command = windowsProcessCommand(pid);
    return isProjectViteCommand(command, projectRoot, process.platform);
  }
  if (process.platform === "darwin") {
    const cwdResult = spawnSync("lsof", ["-a", "-p", String(pid), "-d", "cwd", "-Fn"], { encoding: "utf8" });
    const cwd = cwdResult.stdout?.split("\n").find((line) => line.startsWith("n"))?.slice(1);
    const command = spawnSync("ps", ["-p", String(pid), "-o", "command="], { encoding: "utf8" }).stdout ?? "";
    return Boolean(cwd)
      && resolve(cwd) === projectRoot
      && (command.includes("node_modules/.bin/vite") || command.includes("node_modules/vite/bin/vite.js"));
  }
  if (process.platform !== "linux") return false;
  try {
    const cwd = resolve(readlinkSync(`/proc/${pid}/cwd`));
    const args = readFileSync(`/proc/${pid}/cmdline`).toString("utf8").split("\0").filter(Boolean);
    const vite = args.some((arg) => arg.includes("node_modules/.bin/vite") || arg.includes("node_modules/vite/bin/vite.js"));
    return cwd === projectRoot && vite;
  } catch {
    return false;
  }
}

function windowsProcessCommand(pid) {
  const command = `(Get-CimInstance Win32_Process -Filter 'ProcessId = ${pid}').CommandLine`;
  for (const shell of ["powershell.exe", "pwsh.exe"]) {
    const result = spawnSync(shell, ["-NoProfile", "-NonInteractive", "-Command", command], {
      encoding: "utf8",
      windowsHide: true,
    });
    if (result.status === 0 && result.stdout?.trim()) return result.stdout.trim();
  }
  return "";
}

function portAvailable(host, port) {
  return new Promise((resolveAvailable, reject) => {
    const server = createServer();
    server.unref();
    server.once("error", (error) => {
      if (error.code === "EADDRINUSE") resolveAvailable(false);
      else reject(error);
    });
    server.listen(port, host, () => server.close(() => resolveAvailable(true)));
  });
}

async function waitForPort(host, port, timeoutMs) {
  return waitForStablePortAvailability(
    () => portAvailable(host, port),
    {
      timeoutMs,
      stableMs: PORT_RELEASE_STABLE_MS,
      intervalMs: PORT_RELEASE_POLL_MS,
    },
  );
}
