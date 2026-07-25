import { readFileSync } from "node:fs";
import { createConnection } from "node:net";
import { dirname, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { randomUUID } from "node:crypto";

if (process.platform !== "linux") {
  throw new Error("The SSH server compatibility matrix currently requires a Linux Docker host");
}

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const matrix = JSON.parse(readFileSync(resolve(projectRoot, "tests/compat/ssh-server-matrix.json"), "utf8"));
const healthFaultMatrix = JSON.parse(readFileSync(resolve(projectRoot, "tests/compat/ssh-health-fault-matrix.json"), "utf8"));
if (!Array.isArray(matrix) || !matrix.length) throw new Error("SSH server compatibility matrix is empty");
if (!Array.isArray(healthFaultMatrix) || !healthFaultMatrix.length) throw new Error("SSH health fault matrix is empty");
run("docker", ["info", "--format", "{{.ServerVersion}}"], { quiet: true });

const results = [];
for (const entry of matrix) {
  validateEntry(entry);
  const image = `portmate-compat-${entry.name}:local`;
  const buildArgs = Object.entries(entry.buildArgs ?? {}).flatMap(([name, value]) => ["--build-arg", `${name}=${value}`]);
  run("docker", ["build", "--tag", image, "--file", resolve(projectRoot, entry.dockerfile), ...buildArgs, projectRoot]);

  const container = `portmate-compat-${randomUUID()}`;
  try {
    run("docker", ["run", "--detach", "--rm", "--name", container, "--publish", "127.0.0.1::22", image], { quiet: true });
    const published = run("docker", ["port", container, "22/tcp"], { capture: true }).stdout.trim();
    const port = Number(published.match(/:(\d+)$/)?.[1]);
    if (!Number.isInteger(port) || port <= 0 || port > 65535) {
      throw new Error(`Docker returned an invalid SSH port for ${entry.name}: ${published}`);
    }
    await waitForTcp(port, container, entry.name);
    run("cargo", [
      "test",
      "-p",
      "portmate",
      "external_ssh_server_sftp_scp_compatibility",
      "--",
      "--nocapture",
      "--test-threads=1",
    ], {
      env: {
        ...process.env,
        PORTMATE_COMPAT_SSH_LABEL: entry.name,
        PORTMATE_COMPAT_SSH_HOST: "127.0.0.1",
        PORTMATE_COMPAT_SSH_PORT: String(port),
        PORTMATE_COMPAT_SSH_USERNAME: "portmate",
        PORTMATE_COMPAT_SSH_PASSWORD: "portmate",
      },
    });
    results.push({ name: entry.name, port });
  } finally {
    run("docker", ["rm", "--force", container], { quiet: true, allowFailure: true });
  }
}

const healthFaultImage = "portmate-compat-ssh-health-faults:local";
run("docker", [
  "build",
  "--tag",
  healthFaultImage,
  "--file",
  resolve(projectRoot, "tests/compat/ssh-health-faults-alpine.Dockerfile"),
  projectRoot,
]);
const verifiedHealthFaults = [];
for (const entry of healthFaultMatrix) {
  validateHealthFaultEntry(entry);
  const container = `portmate-compat-${randomUUID()}`;
  try {
    run("docker", [
      "run",
      "--detach",
      "--rm",
      "--name",
      container,
      "--env",
      `PORTMATE_SSH_HEALTH_FAULT=${entry.name}`,
      "--publish",
      "127.0.0.1::22",
      healthFaultImage,
    ], { quiet: true });
    const published = run("docker", ["port", container, "22/tcp"], { capture: true }).stdout.trim();
    const port = Number(published.match(/:(\d+)$/)?.[1]);
    if (!Number.isInteger(port) || port <= 0 || port > 65535) {
      throw new Error(`Docker returned an invalid SSH health fault port for ${entry.name}: ${published}`);
    }
    await waitForTcp(port, container, entry.name);
    run("cargo", [
      "test",
      "-p",
      "portmate",
      "external_ssh_health_fault_matrix_case",
      "--",
      "--nocapture",
      "--test-threads=1",
    ], {
      env: {
        ...process.env,
        PORTMATE_COMPAT_SSH_HEALTH_FAULT: entry.name,
        PORTMATE_COMPAT_SSH_HOST: "127.0.0.1",
        PORTMATE_COMPAT_SSH_PORT: String(port),
        PORTMATE_COMPAT_SSH_USERNAME: "portmate",
        PORTMATE_COMPAT_SSH_PASSWORD: "portmate",
        PORTMATE_COMPAT_SSH_CONTAINER: container,
        PORTMATE_COMPAT_SSH_PROBE_SFTP: String(entry.probeSftp),
        PORTMATE_COMPAT_SSH_EXPECTED_STATUS: entry.expectedStatus ?? "",
        PORTMATE_COMPAT_SSH_EXPECTED_ERROR_FIELD: entry.expectedErrorField ?? "",
        PORTMATE_COMPAT_SSH_EXPECTED_ERROR_CONTAINS: entry.expectedErrorContains ?? "",
      },
    });
    verifiedHealthFaults.push(entry.name);
  } finally {
    run("docker", ["unpause", container], { quiet: true, allowFailure: true });
    run("docker", ["rm", "--force", container], { quiet: true, allowFailure: true });
  }
}

console.log(JSON.stringify({
  verifiedServers: results.map(({ name }) => name),
  verifiedHealthFaults,
}, null, 2));

function validateEntry(entry) {
  if (!entry || typeof entry.name !== "string" || !/^[a-z0-9.-]+$/.test(entry.name)) {
    throw new Error(`Invalid SSH compatibility matrix entry: ${JSON.stringify(entry)}`);
  }
  if (typeof entry.dockerfile !== "string" || !entry.dockerfile.startsWith("tests/compat/")) {
    throw new Error(`Invalid SSH compatibility Dockerfile: ${JSON.stringify(entry)}`);
  }
  for (const [name, value] of Object.entries(entry.buildArgs ?? {})) {
    if (!/^[A-Z][A-Z0-9_]*$/.test(name) || typeof value !== "string" || !/^[a-zA-Z0-9._-]+$/.test(value)) {
      throw new Error(`Invalid Docker build argument in ${entry.name}`);
    }
  }
}

function validateHealthFaultEntry(entry) {
  if (!entry || typeof entry.name !== "string" || !/^[a-z0-9-]+$/.test(entry.name)) {
    throw new Error(`Invalid SSH health fault entry: ${JSON.stringify(entry)}`);
  }
  if (typeof entry.probeSftp !== "boolean") {
    throw new Error(`Invalid SSH health SFTP flag: ${JSON.stringify(entry)}`);
  }
  if (entry.expectedStatus !== undefined && !["degraded", "unresponsive"].includes(entry.expectedStatus)) {
    throw new Error(`Invalid SSH health expected status: ${JSON.stringify(entry)}`);
  }
  if (entry.expectedErrorField !== undefined && !["transportError", "channelError", "sftpError"].includes(entry.expectedErrorField)) {
    throw new Error(`Invalid SSH health error field: ${JSON.stringify(entry)}`);
  }
  const expectsReport = entry.expectedStatus !== undefined && entry.expectedErrorField !== undefined;
  const expectsError = typeof entry.expectedErrorContains === "string" && entry.expectedErrorContains.length > 0;
  if (expectsReport === expectsError) {
    throw new Error(`SSH health fault entry must expect exactly one report or command error: ${JSON.stringify(entry)}`);
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
