import { randomUUID } from "node:crypto";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  compatibilityUsesCachedImages,
  filterCompatibilityEntries,
  prepareCompatibilityImage,
} from "./compat-docker-images.mjs";

if (process.platform !== "linux") {
  throw new Error("The TCP/Telnet server compatibility matrix currently requires a Linux Docker host");
}

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const dockerControlTimeoutMs = 180_000;
const useCachedImages = compatibilityUsesCachedImages();
const matrixPath = resolve(projectRoot, "tests/compat/tcp-telnet-server-matrix.json");
const allEntries = JSON.parse(readFileSync(matrixPath, "utf8"));
if (!Array.isArray(allEntries) || !allEntries.length) {
  throw new Error("TCP/Telnet server compatibility matrix is empty");
}
allEntries.forEach(validateEntry);
const matrix = filterCompatibilityEntries(allEntries);
run("docker", ["info", "--format", "{{.ServerVersion}}"], { quiet: true });

const verifiedServers = [];
for (const entry of matrix) {
  const image = `portmate-compat-${entry.name}:local`;
  const buildArgs = Object.entries(entry.buildArgs ?? {}).flatMap(([name, value]) => ["--build-arg", `${name}=${value}`]);
  await prepareCompatibilityImage({
    run,
    image,
    useCachedImages,
    buildArgs: ["build", "--tag", image, "--file", resolve(projectRoot, entry.dockerfile), ...buildArgs, projectRoot],
  });

  const container = `portmate-compat-${randomUUID()}`;
  try {
    const containerEnv = Object.entries(entry.containerEnv ?? {}).flatMap(([name, value]) => ["--env", `${name}=${value}`]);
    run("docker", [
      "run",
      "--detach",
      "--name",
      container,
      "--publish",
      "127.0.0.1::23",
      ...containerEnv,
      image,
    ], { quiet: true, timeout: dockerControlTimeoutMs });
    const port = await waitForPublishedPort(container, entry.name);
    await waitForListeningSocket(container, entry.name);
    run("cargo", [
      "test",
      "--locked",
      "-p",
      "portmate",
      "external_tcp_telnet_server_compatibility",
      "--",
      "--nocapture",
      "--test-threads=1",
    ], {
      env: {
        ...process.env,
        PORTMATE_COMPAT_SOCKET_LABEL: entry.name,
        PORTMATE_COMPAT_SOCKET_HOST: "127.0.0.1",
        PORTMATE_COMPAT_SOCKET_PORT: String(port),
        PORTMATE_COMPAT_SOCKET_PROTOCOL: entry.protocol,
        PORTMATE_COMPAT_SOCKET_MODE: entry.mode,
        PORTMATE_COMPAT_SOCKET_TLS: String(entry.tls ?? false),
        PORTMATE_COMPAT_SOCKET_TLS_SERVER_NAME: entry.tlsServerName ?? "",
        PORTMATE_COMPAT_SOCKET_TLS_ACCEPT_INVALID_CERT: String(entry.tlsAcceptInvalidCert ?? false),
        PORTMATE_COMPAT_SOCKET_EXPECT_REJECTED_OPTION: entry.expectRejectedTelnetOption === undefined
          ? ""
          : String(entry.expectRejectedTelnetOption),
        PORTMATE_COMPAT_SOCKET_VERIFY_NAWS_PTY: String(entry.verifyNawsPty ?? false),
      },
    });
    verifiedServers.push(entry.name);
  } finally {
    run("docker", ["rm", "--force", container], {
      quiet: true,
      allowFailure: true,
      timeout: dockerControlTimeoutMs,
    });
  }
}

console.log(JSON.stringify({ verifiedServers }, null, 2));

function validateEntry(entry) {
  if (!entry || typeof entry.name !== "string" || !/^[a-z0-9.-]+$/.test(entry.name)) {
    throw new Error(`Invalid TCP/Telnet compatibility matrix entry: ${JSON.stringify(entry)}`);
  }
  if (typeof entry.dockerfile !== "string" || !entry.dockerfile.startsWith("tests/compat/")) {
    throw new Error(`Invalid compatibility Dockerfile: ${JSON.stringify(entry)}`);
  }
  if (!new Set(["tcp", "telnet"]).has(entry.protocol) || typeof entry.mode !== "string") {
    throw new Error(`Invalid compatibility protocol or mode in ${entry.name}`);
  }
  if ((entry.tls !== undefined && typeof entry.tls !== "boolean")
    || (entry.tlsAcceptInvalidCert !== undefined && typeof entry.tlsAcceptInvalidCert !== "boolean")
    || (entry.tlsServerName !== undefined
      && (typeof entry.tlsServerName !== "string" || !/^[a-zA-Z0-9.-]+$/.test(entry.tlsServerName)))) {
    throw new Error(`Invalid TLS compatibility setting in ${entry.name}`);
  }
  if (entry.expectRejectedTelnetOption !== undefined
    && (!Number.isInteger(entry.expectRejectedTelnetOption)
      || entry.expectRejectedTelnetOption < 0
      || entry.expectRejectedTelnetOption > 255
      || entry.protocol !== "telnet")) {
    throw new Error(`Invalid rejected Telnet option in ${entry.name}`);
  }
  if (entry.verifyNawsPty !== undefined
    && (typeof entry.verifyNawsPty !== "boolean"
      || entry.protocol !== "telnet"
      || entry.mode !== "shell")) {
    throw new Error(`Invalid NAWS PTY verification setting in ${entry.name}`);
  }
  for (const values of [entry.buildArgs ?? {}, entry.containerEnv ?? {}]) {
    for (const [name, value] of Object.entries(values)) {
      if (!/^[A-Z][A-Z0-9_]*$/.test(name) || typeof value !== "string" || !/^[a-zA-Z0-9._-]+$/.test(value)) {
        throw new Error(`Invalid Docker setting in ${entry.name}`);
      }
    }
  }
}

async function waitForPublishedPort(container, label) {
  let lastState = "not inspectable";
  for (let attempt = 0; attempt < 200; attempt += 1) {
    const inspected = run("docker", ["inspect", container], {
      capture: true,
      allowFailure: true,
      timeout: dockerControlTimeoutMs,
    });
    if (inspected.status === 0) {
      const details = JSON.parse(inspected.stdout)[0];
      const state = details?.State ?? {};
      lastState = state.Status ?? "unknown";
      if (["dead", "exited", "removing"].includes(lastState) || state.Error) {
        throwContainerStartupError(container, label, lastState, state.Error);
      }
      const published = details?.NetworkSettings?.Ports?.["23/tcp"]?.find(({ HostIp }) => HostIp === "127.0.0.1")
        ?? details?.NetworkSettings?.Ports?.["23/tcp"]?.[0];
      if (state.Running && published?.HostPort) {
        const port = Number(published.HostPort);
        if (!Number.isInteger(port) || port <= 0 || port > 65535) {
          throw new Error(`Docker returned an invalid socket port for ${label}: ${published.HostPort}`);
        }
        return port;
      }
    }
    await new Promise((resolveWait) => setTimeout(resolveWait, 50));
  }
  throw new Error(`timed out waiting for Docker to publish ${label} socket port (last state: ${lastState})`);
}

async function waitForListeningSocket(container, label) {
  const checkListen = "awk '$2 ~ /:0017$/ && $4 == \"0A\" { found = 1 } END { exit found ? 0 : 1 }' /proc/net/tcp /proc/net/tcp6";
  for (let attempt = 0; attempt < 200; attempt += 1) {
    const running = run("docker", ["inspect", "--format", "{{.State.Running}}", container], {
      capture: true,
      allowFailure: true,
      timeout: dockerControlTimeoutMs,
    });
    if (running.status !== 0 || running.stdout.trim() !== "true") {
      throwContainerStartupError(container, label, "not running", "");
    }
    const listening = run("docker", ["exec", container, "sh", "-lc", checkListen], {
      capture: true,
      allowFailure: true,
      timeout: dockerControlTimeoutMs,
    });
    if (listening.status === 0) return;
    await new Promise((resolveWait) => setTimeout(resolveWait, 50));
  }
  throw new Error(`timed out waiting for ${label} to listen on container port 23`);
}

function throwContainerStartupError(container, label, state, startError) {
  const logs = run("docker", ["logs", container], { capture: true, allowFailure: true });
  throw new Error(`${label} container failed during startup (${state}: ${startError || "no start error"})\n${logs.stdout}\n${logs.stderr}`);
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    env: options.env ?? process.env,
    encoding: "utf8",
    stdio: options.capture || options.quiet ? ["ignore", "pipe", "pipe"] : "inherit",
    maxBuffer: 16 * 1024 * 1024,
    timeout: options.timeout ?? 300_000,
  });
  if (result.error && !options.allowFailure) {
    if (result.error.code === "ETIMEDOUT") {
      throw new Error(`${command} ${args.join(" ")} exceeded its ${options.timeout ?? 300_000} ms timeout`);
    }
    throw result.error;
  }
  if (result.status !== 0 && !options.allowFailure) {
    const details = [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
    throw new Error(`${command} ${args.join(" ")} failed with exit code ${result.status ?? 1}${details ? `\n${details}` : ""}`);
  }
  return result;
}
