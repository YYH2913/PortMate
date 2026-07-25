import { randomUUID } from "node:crypto";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { createConnection } from "node:net";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

if (process.platform !== "linux") {
  throw new Error("The TCP/Telnet server compatibility matrix currently requires a Linux Docker host");
}

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const matrixPath = resolve(projectRoot, "tests/compat/tcp-telnet-server-matrix.json");
const matrix = JSON.parse(readFileSync(matrixPath, "utf8"));
if (!Array.isArray(matrix) || !matrix.length) {
  throw new Error("TCP/Telnet server compatibility matrix is empty");
}
run("docker", ["info", "--format", "{{.ServerVersion}}"], { quiet: true });

const verifiedServers = [];
for (const entry of matrix) {
  validateEntry(entry);
  const image = `portmate-compat-${entry.name}:local`;
  const buildArgs = Object.entries(entry.buildArgs ?? {}).flatMap(([name, value]) => ["--build-arg", `${name}=${value}`]);
  run("docker", ["build", "--tag", image, "--file", resolve(projectRoot, entry.dockerfile), ...buildArgs, projectRoot]);

  const container = `portmate-compat-${randomUUID()}`;
  try {
    const containerEnv = Object.entries(entry.containerEnv ?? {}).flatMap(([name, value]) => ["--env", `${name}=${value}`]);
    run("docker", [
      "run",
      "--detach",
      "--rm",
      "--name",
      container,
      "--publish",
      "127.0.0.1::23",
      ...containerEnv,
      image,
    ], { quiet: true });
    const published = run("docker", ["port", container, "23/tcp"], { capture: true }).stdout.trim();
    const port = Number(published.match(/:(\d+)$/)?.[1]);
    if (!Number.isInteger(port) || port <= 0 || port > 65535) {
      throw new Error(`Docker returned an invalid socket port for ${entry.name}: ${published}`);
    }
    await waitForTcp(port, container, entry.name);
    run("cargo", [
      "test",
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
      },
    });
    verifiedServers.push(entry.name);
  } finally {
    run("docker", ["rm", "--force", container], { quiet: true, allowFailure: true });
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
  for (const values of [entry.buildArgs ?? {}, entry.containerEnv ?? {}]) {
    for (const [name, value] of Object.entries(values)) {
      if (!/^[A-Z][A-Z0-9_]*$/.test(name) || typeof value !== "string" || !/^[a-zA-Z0-9._-]+$/.test(value)) {
        throw new Error(`Invalid Docker setting in ${entry.name}`);
      }
    }
  }
}

async function waitForTcp(port, container, label) {
  for (let attempt = 0; attempt < 200; attempt += 1) {
    const running = run("docker", ["inspect", "--format", "{{.State.Running}}", container], {
      capture: true,
      allowFailure: true,
    });
    if (running.status !== 0 || running.stdout.trim() !== "true") {
      const logs = run("docker", ["logs", container], { capture: true, allowFailure: true });
      throw new Error(`${label} container exited during startup\n${logs.stdout}\n${logs.stderr}`);
    }
    if (await tcpConnects(port)) return;
    await new Promise((resolveWait) => setTimeout(resolveWait, 50));
  }
  throw new Error(`timed out waiting for ${label} on 127.0.0.1:${port}`);
}

function tcpConnects(port) {
  return new Promise((resolveConnect) => {
    const socket = createConnection({ host: "127.0.0.1", port });
    socket.setTimeout(250);
    socket.once("connect", () => {
      socket.destroy();
      resolveConnect(true);
    });
    socket.once("timeout", () => {
      socket.destroy();
      resolveConnect(false);
    });
    socket.once("error", () => resolveConnect(false));
  });
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    env: options.env ?? process.env,
    encoding: "utf8",
    stdio: options.capture || options.quiet ? ["ignore", "pipe", "pipe"] : "inherit",
    maxBuffer: 16 * 1024 * 1024,
  });
  if (result.error && !options.allowFailure) throw result.error;
  if (result.status !== 0 && !options.allowFailure) {
    const details = [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
    throw new Error(`${command} ${args.join(" ")} failed with exit code ${result.status ?? 1}${details ? `\n${details}` : ""}`);
  }
  return result;
}
