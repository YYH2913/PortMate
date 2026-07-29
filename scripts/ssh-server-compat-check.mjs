import { readFileSync } from "node:fs";
import { createConnection } from "node:net";
import { dirname, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { randomUUID } from "node:crypto";
import {
  compatibilityUsesCachedImages,
  prepareCompatibilityImage,
} from "./compat-docker-images.mjs";

if (process.platform !== "linux") {
  throw new Error("The SSH server compatibility matrix currently requires a Linux Docker host");
}

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const dockerControlTimeoutMs = 180_000;
const useCachedImages = compatibilityUsesCachedImages();
const matrix = JSON.parse(readFileSync(resolve(projectRoot, "tests/compat/ssh-server-matrix.json"), "utf8"));
const healthFaultMatrix = JSON.parse(readFileSync(resolve(projectRoot, "tests/compat/ssh-health-fault-matrix.json"), "utf8"));
const transferFaultMatrix = JSON.parse(readFileSync(resolve(projectRoot, "tests/compat/ssh-transfer-fault-matrix.json"), "utf8"));
if (!Array.isArray(matrix) || !matrix.length) throw new Error("SSH server compatibility matrix is empty");
if (!Array.isArray(healthFaultMatrix) || !healthFaultMatrix.length) throw new Error("SSH health fault matrix is empty");
if (!Array.isArray(transferFaultMatrix) || !transferFaultMatrix.length) throw new Error("SSH transfer fault matrix is empty");
run("docker", ["info", "--format", "{{.ServerVersion}}"], { quiet: true });

const results = [];
for (const entry of matrix) {
  validateEntry(entry);
  const image = `portmate-compat-${entry.name}:local`;
  const buildArgs = Object.entries(entry.buildArgs ?? {}).flatMap(([name, value]) => ["--build-arg", `${name}=${value}`]);
  await prepareCompatibilityImage({
    run,
    image,
    useCachedImages,
    buildArgs: ["build", "--tag", image, "--file", resolve(projectRoot, entry.dockerfile), ...buildArgs, projectRoot],
    inspectOptions: { timeout: dockerControlTimeoutMs },
  });

  const container = `portmate-compat-${randomUUID()}`;
  try {
    run("docker", ["run", "--detach", "--name", container, "--publish", "127.0.0.1::22", image], {
      quiet: true,
      timeout: dockerControlTimeoutMs,
    });
    const port = await waitForPublishedPort(container, entry.name);
    await waitForTcp(port, container, entry.name);
    const compatibilityTest = (entry.protocols ?? ["sftp", "scp"]).length === 1
      && entry.protocols[0] === "sftp"
      ? "external_sftp_server_compatibility"
      : "external_ssh_server_sftp_scp_compatibility";
    run("cargo", [
      "test",
      "-p",
      "portmate",
      compatibilityTest,
      "--",
      "--nocapture",
      "--test-threads=1",
    ], {
      env: {
        ...process.env,
        PORTMATE_COMPAT_SSH_LABEL: entry.name,
        PORTMATE_COMPAT_SSH_PROTOCOLS: (entry.protocols ?? ["sftp", "scp"]).join(","),
        PORTMATE_COMPAT_SSH_HOST: "127.0.0.1",
        PORTMATE_COMPAT_SSH_PORT: String(port),
        PORTMATE_COMPAT_SSH_USERNAME: "portmate",
        PORTMATE_COMPAT_SSH_PASSWORD: "portmate",
        PORTMATE_COMPAT_SFTP_EXTENDED_STATUS_CODES: entry.verifyExtendedSftpStatusCodes ? "1" : "0",
      },
    });
    if (entry.runActiveTransferDisconnect !== false) {
      run("cargo", [
        "test",
        "-p",
        "portmate",
        "external_ssh_server_active_transfer_disconnect",
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
          PORTMATE_COMPAT_SSH_CONTAINER: container,
          PORTMATE_COMPAT_SSH_DISCONNECT_PROTOCOL: entry.disconnectProtocol,
          PORTMATE_COMPAT_SSH_MODEM_DISCONNECT_PROTOCOL: entry.modemDisconnectProtocol,
        },
      });
    }
    results.push({
      name: entry.name,
      port,
      protocols: entry.protocols ?? ["sftp", "scp"],
      activeTransferDisconnect: entry.runActiveTransferDisconnect === false
        ? null
        : entry.disconnectProtocol,
      activeModemTransferDisconnect: entry.runActiveTransferDisconnect === false
        ? null
        : entry.modemDisconnectProtocol,
      extendedSftpStatusCodes: entry.verifyExtendedSftpStatusCodes === true,
    });
  } finally {
    run("docker", ["rm", "--force", container], { quiet: true, allowFailure: true, timeout: dockerControlTimeoutMs });
  }
}

const healthFaultImage = "portmate-compat-ssh-health-faults:local";
await prepareCompatibilityImage({
  run,
  image: healthFaultImage,
  useCachedImages,
  buildArgs: [
    "build",
    "--tag",
    healthFaultImage,
    "--file",
    resolve(projectRoot, "tests/compat/ssh-health-faults-alpine.Dockerfile"),
    projectRoot,
  ],
  inspectOptions: { timeout: dockerControlTimeoutMs },
});
const verifiedHealthFaults = [];
for (const entry of healthFaultMatrix) {
  validateHealthFaultEntry(entry);
  const container = `portmate-compat-${randomUUID()}`;
  try {
    run("docker", [
      "run",
      "--detach",
      "--name",
      container,
      "--env",
      `PORTMATE_SSH_HEALTH_FAULT=${entry.name}`,
      "--publish",
      "127.0.0.1::22",
      healthFaultImage,
    ], {
      quiet: true,
      timeout: dockerControlTimeoutMs,
    });
    const port = await waitForPublishedPort(container, entry.name);
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
        PORTMATE_COMPAT_SSH_EXPECT_SFTP_RECOVERY: String(entry.expectSftpRecovery ?? false),
      },
    });
    verifiedHealthFaults.push(entry.name);
  } finally {
    run("docker", ["unpause", container], { quiet: true, allowFailure: true, timeout: dockerControlTimeoutMs });
    run("docker", ["rm", "--force", container], { quiet: true, allowFailure: true, timeout: dockerControlTimeoutMs });
  }
}

const verifiedTransferFaults = [];
for (const entry of transferFaultMatrix) {
  validateTransferFaultEntry(entry);
  const container = `portmate-compat-${randomUUID()}`;
  try {
    run("docker", [
      "run",
      "--detach",
      "--name",
      container,
      "--env",
      `PORTMATE_SSH_HEALTH_FAULT=${entry.name}`,
      "--publish",
      "127.0.0.1::22",
      healthFaultImage,
    ], {
      quiet: true,
      timeout: dockerControlTimeoutMs,
    });
    const port = await waitForPublishedPort(container, entry.name);
    await waitForTcp(port, container, entry.name);
    run("cargo", [
      "test",
      "-p",
      "portmate",
      "external_ssh_transfer_fault_matrix_case",
      "--",
      "--nocapture",
      "--test-threads=1",
    ], {
      env: {
        ...process.env,
        PORTMATE_COMPAT_SSH_TRANSFER_FAULT: entry.name,
        PORTMATE_COMPAT_SSH_HOST: "127.0.0.1",
        PORTMATE_COMPAT_SSH_PORT: String(port),
        PORTMATE_COMPAT_SSH_USERNAME: "portmate",
        PORTMATE_COMPAT_SSH_PASSWORD: "portmate",
        PORTMATE_COMPAT_SSH_TRANSFER_PROTOCOL: entry.protocol,
        PORTMATE_COMPAT_SSH_TRANSFER_EXPECTED_ERROR: entry.expectedErrorContains,
      },
    });
    verifiedTransferFaults.push(entry.name);
  } finally {
    run("docker", ["rm", "--force", container], { quiet: true, allowFailure: true, timeout: dockerControlTimeoutMs });
  }
}

console.log(JSON.stringify({
  verifiedServers: results.map(({ name }) => name),
  verifiedActiveTransferDisconnects: results
    .filter(({ activeTransferDisconnect }) => activeTransferDisconnect)
    .map(({ name, activeTransferDisconnect }) => ({
    name,
    protocol: activeTransferDisconnect,
    })),
  verifiedActiveModemTransferDisconnects: results
    .filter(({ activeModemTransferDisconnect }) => activeModemTransferDisconnect)
    .map(({ name, activeModemTransferDisconnect }) => ({
    name,
    protocol: activeModemTransferDisconnect,
    })),
  verifiedExtendedSftpStatusServers: results
    .filter(({ extendedSftpStatusCodes }) => extendedSftpStatusCodes)
    .map(({ name }) => name),
  verifiedHealthFaults,
  verifiedTransferFaults,
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
  const protocols = entry.protocols ?? ["sftp", "scp"];
  if (!Array.isArray(protocols) || !protocols.length || protocols.some((protocol) => !["sftp", "scp"].includes(protocol))) {
    throw new Error(`Invalid SSH compatibility protocols in ${entry.name}`);
  }
  if (entry.runActiveTransferDisconnect !== undefined && typeof entry.runActiveTransferDisconnect !== "boolean") {
    throw new Error(`Invalid SSH compatibility active-transfer flag in ${entry.name}`);
  }
  if (entry.verifyExtendedSftpStatusCodes !== undefined && typeof entry.verifyExtendedSftpStatusCodes !== "boolean") {
    throw new Error(`Invalid SSH compatibility extended-status flag in ${entry.name}`);
  }
  if (entry.verifyExtendedSftpStatusCodes === true && !protocols.includes("sftp")) {
    throw new Error(`SSH compatibility extended-status checks require SFTP in ${entry.name}`);
  }
  if (entry.runActiveTransferDisconnect !== false) {
    if (!entry.disconnectProtocol || !["sftp", "scp"].includes(entry.disconnectProtocol)) {
      throw new Error(`Invalid SSH compatibility disconnect protocol: ${JSON.stringify(entry)}`);
    }
    if (!entry.modemDisconnectProtocol || !["xmodem", "ymodem", "zmodem"].includes(entry.modemDisconnectProtocol)) {
      throw new Error(`Invalid SSH compatibility modem disconnect protocol: ${JSON.stringify(entry)}`);
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
  const hasPartialReportExpectation = (entry.expectedStatus !== undefined) !== (entry.expectedErrorField !== undefined);
  const expectedErrorContains = entry.expectedErrorContains;
  if (hasPartialReportExpectation
    || (!expectsReport && (typeof expectedErrorContains !== "string" || expectedErrorContains.length === 0))) {
    throw new Error(`SSH health fault entry must expect exactly one report or command error: ${JSON.stringify(entry)}`);
  }
  if (expectedErrorContains !== undefined && (typeof expectedErrorContains !== "string" || expectedErrorContains.length === 0)) {
    throw new Error(`Invalid SSH health error substring: ${JSON.stringify(entry)}`);
  }
  if (entry.expectSftpRecovery !== undefined
    && (typeof entry.expectSftpRecovery !== "boolean"
      || !entry.probeSftp
      || !expectsReport
      || entry.expectedErrorField !== "sftpError")) {
    throw new Error(`Invalid SSH health SFTP recovery expectation: ${JSON.stringify(entry)}`);
  }
}

function validateTransferFaultEntry(entry) {
  if (!entry || typeof entry.name !== "string" || !/^[a-z0-9-]+$/.test(entry.name)) {
    throw new Error(`Invalid SSH transfer fault entry: ${JSON.stringify(entry)}`);
  }
  if (!["sftp", "scp"].includes(entry.protocol)) {
    throw new Error(`Invalid SSH transfer fault protocol: ${JSON.stringify(entry)}`);
  }
  if (typeof entry.expectedErrorContains !== "string" || entry.expectedErrorContains.length === 0) {
    throw new Error(`Invalid SSH transfer fault expectation: ${JSON.stringify(entry)}`);
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
        const logs = run("docker", ["logs", container], { capture: true, allowFailure: true });
        throw new Error(`${label} container failed during startup (${lastState}: ${state.Error || "no start error"})\n${logs.stdout}\n${logs.stderr}`);
      }

      const published = details?.NetworkSettings?.Ports?.["22/tcp"]?.find(({ HostIp }) => HostIp === "127.0.0.1")
        ?? details?.NetworkSettings?.Ports?.["22/tcp"]?.[0];
      if (state.Running && published?.HostPort) {
        const port = Number(published.HostPort);
        if (!Number.isInteger(port) || port <= 0 || port > 65535) {
          throw new Error(`Docker returned an invalid SSH port for ${label}: ${published.HostPort}`);
        }
        return port;
      }
    }
    await new Promise((resolveWait) => setTimeout(resolveWait, 50));
  }
  throw new Error(`timed out waiting for Docker to publish ${label} SSH port (last state: ${lastState})`);
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
    if (await sshBannerReady(port)) return;
    await new Promise((resolveWait) => setTimeout(resolveWait, 50));
  }
  throw new Error(`timed out waiting for ${label} on 127.0.0.1:${port}`);
}

function sshBannerReady(port) {
  return new Promise((resolveConnect) => {
    const socket = createConnection({ host: "127.0.0.1", port });
    let settled = false;
    let received = Buffer.alloc(0);
    const finish = (ready) => {
      if (settled) return;
      settled = true;
      socket.destroy();
      resolveConnect(ready);
    };
    socket.setTimeout(500);
    socket.on("data", (chunk) => {
      received = Buffer.concat([received, chunk], Math.min(1024, received.length + chunk.length));
      const text = received.toString("ascii");
      if (text.split(/\r?\n/).some((line) => line.startsWith("SSH-"))) {
        finish(true);
      } else if (received.length >= 1024) {
        finish(false);
      }
    });
    socket.once("timeout", () => finish(false));
    socket.once("error", () => finish(false));
    socket.once("close", () => finish(false));
  });
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
