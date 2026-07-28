import { randomUUID } from "node:crypto";
import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createConnection } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import {
  compatibilityUsesCachedImages,
  prepareCompatibilityImage,
} from "./compat-docker-images.mjs";

if (process.platform !== "linux") {
  throw new Error("The SSH GSSAPI compatibility matrix requires Linux");
}

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const controlTimeoutMs = 180_000;
const cargoTargetDir = process.env.PORTMATE_GSSAPI_TARGET_DIR
  ? resolve(process.env.PORTMATE_GSSAPI_TARGET_DIR)
  : resolve(projectRoot, "target/gssapi-compat");
const baseEnvironment = {
  ...process.env,
  CARGO_TARGET_DIR: cargoTargetDir,
};
const matrix = JSON.parse(readFileSync(
  resolve(projectRoot, "tests/compat/gssapi-server-matrix.json"),
  "utf8",
));
if (!Array.isArray(matrix) || matrix.length < 3) {
  throw new Error("SSH GSSAPI server matrix must contain at least three entries");
}
const matrixNames = new Set();
for (const entry of matrix) {
  validateMatrixEntry(entry);
  if (matrixNames.has(entry.name)) {
    throw new Error(`duplicate SSH GSSAPI matrix name: ${entry.name}`);
  }
  matrixNames.add(entry.name);
}
const useCachedImages = compatibilityUsesCachedImages();
const temporaryRoot = mkdtempSync(join(tmpdir(), "portmate-gssapi-compat-"));
const verifiedServers = [];

try {
  run("docker", ["info", "--format", "{{.ServerVersion}}"], { quiet: true });
  run("bash", ["scripts/libssh-gssapi-build-check.sh"], {
    env: baseEnvironment,
    timeout: 600_000,
  });
  for (const entry of matrix) {
    const image = `portmate-compat-gssapi-${entry.name}:local`;
    await prepareCompatibilityImage({
      run,
      image,
      useCachedImages,
      buildArgs: [
        "build",
        "--tag",
        image,
        "--file",
        resolve(projectRoot, "tests/compat/gssapi-openssh.Dockerfile"),
        "--build-arg",
        `PORTMATE_GSSAPI_BASE_IMAGE=${entry.baseImage}`,
        projectRoot,
      ],
      buildOptions: { timeout: 600_000 },
      inspectOptions: { timeout: controlTimeoutMs },
    });

    const cases = [];
    const version = await withServer(image, "yes", async (server) => {
      const kerberos = configureKerberos(server);
      acquireTicket(server, kerberos);
      runCase("success", server, kerberos, cases);
      runCase("gssapi-preferred", server, kerberos, cases);
      runCase("host-key-reject", server, kerberos, cases);
      destroyTicket(kerberos);
      runCase("no-ticket", server, kerberos, cases);
      runCase("password-fallback", server, kerberos, cases);
      return server.version;
    });

    await withServer(image, "no", async (server) => {
      const kerberos = configureKerberos(server);
      acquireTicket(server, kerberos);
      runCase("server-disabled", server, kerberos, cases);
      runCase("server-disabled-password-fallback", server, kerberos, cases);
      destroyTicket(kerberos);
    });
    verifiedServers.push({ name: entry.name, version, cases });
  }

  console.log(JSON.stringify({ verifiedGssapiServers: verifiedServers }, null, 2));
} finally {
  rmSync(temporaryRoot, { recursive: true, force: true });
}

async function withServer(image, gssapiAuthentication, callback) {
  const container = `portmate-gssapi-${randomUUID()}`;
  try {
    run("docker", [
      "run",
      "--detach",
      "--name",
      container,
      "--hostname",
      "localhost",
      "--env",
      `PORTMATE_GSSAPI_AUTH=${gssapiAuthentication}`,
      "--publish",
      "127.0.0.1::22/tcp",
      "--publish",
      "127.0.0.1::88/tcp",
      image,
    ], { quiet: true, timeout: controlTimeoutMs });
    const ports = await waitForPublishedPorts(container);
    await waitForTcp(ports.kdc, container, "Kerberos KDC", false);
    await waitForTcp(ports.ssh, container, "OpenSSH", true);
    const versionResult = run("docker", ["exec", container, "/usr/sbin/sshd", "-V"], {
      capture: true,
      timeout: controlTimeoutMs,
    });
    const version = `${versionResult.stdout}${versionResult.stderr}`.trim();
    if (!/^OpenSSH_[0-9]/.test(version)) {
      throw new Error(`invalid OpenSSH version from ${container}: ${JSON.stringify(version)}`);
    }
    try {
      return await callback({ container, version, ...ports });
    } catch (error) {
      throw containerFailure(container, error instanceof Error ? error.message : String(error));
    }
  } finally {
    run("docker", ["rm", "--force", container], {
      quiet: true,
      allowFailure: true,
      timeout: controlTimeoutMs,
    });
  }
}

function configureKerberos(server) {
  const suffix = randomUUID();
  const config = join(temporaryRoot, `krb5-${suffix}.conf`);
  const cache = join(temporaryRoot, `ccache-${suffix}`);
  writeFileSync(config, `[libdefaults]
    default_realm = PORTMATE.TEST
    dns_lookup_kdc = false
    dns_lookup_realm = false
    rdns = false
    udp_preference_limit = 1

[realms]
    PORTMATE.TEST = {
        kdc = 127.0.0.1:${server.kdc}
    }

[domain_realm]
    localhost = PORTMATE.TEST
    .localhost = PORTMATE.TEST
`, { mode: 0o600 });
  return {
    env: {
      ...baseEnvironment,
      KRB5_CONFIG: config,
      KRB5CCNAME: `FILE:${cache}`,
      KRB5_TRACE: "/dev/stderr",
    },
    cache,
    remoteCache: `/tmp/portmate-client-${suffix}.ccache`,
  };
}

function acquireTicket(server, kerberos) {
  run("docker", [
    "exec",
    server.container,
    "env",
    `KRB5CCNAME=FILE:${kerberos.remoteCache}`,
    "kinit",
    "-k",
    "-t",
    "/portmate-client.keytab",
    "portmate@PORTMATE.TEST",
  ], { quiet: true, timeout: controlTimeoutMs });
  run("docker", ["cp", `${server.container}:${kerberos.remoteCache}`, kerberos.cache], {
    quiet: true,
    timeout: controlTimeoutMs,
  });
  chmodSync(kerberos.cache, 0o600);
}

function destroyTicket(kerberos) {
  rmSync(kerberos.cache, { force: true });
}

function runCase(name, server, kerberos, cases) {
  run("cargo", [
    "test",
    "-p",
    "portmate",
    "external_ssh_gssapi_runtime_matrix_case",
    "--",
    "--nocapture",
    "--test-threads=1",
  ], {
    env: {
      ...kerberos.env,
      PORTMATE_COMPAT_GSSAPI_CASE: name,
      PORTMATE_COMPAT_GSSAPI_HOST: "localhost",
      PORTMATE_COMPAT_GSSAPI_PORT: String(server.ssh),
      PORTMATE_COMPAT_LIBSSH_TRACE: "1",
    },
    timeout: 600_000,
  });
  cases.push(name);
}

function validateMatrixEntry(entry) {
  if (!entry || typeof entry.name !== "string" || !/^[a-z0-9.-]+$/.test(entry.name)) {
    throw new Error(`invalid SSH GSSAPI matrix name: ${JSON.stringify(entry)}`);
  }
  if (typeof entry.baseImage !== "string" || !/^[a-z0-9./:-]+$/.test(entry.baseImage)) {
    throw new Error(`invalid SSH GSSAPI base image for ${entry.name}`);
  }
}

async function waitForPublishedPorts(container) {
  for (let attempt = 0; attempt < 200; attempt += 1) {
    const inspected = run("docker", ["inspect", container], {
      capture: true,
      allowFailure: true,
      timeout: controlTimeoutMs,
    });
    if (inspected.status === 0) {
      const details = JSON.parse(inspected.stdout)[0];
      if (details?.State?.Running) {
        const ssh = publishedPort(details, "22/tcp");
        const kdc = publishedPort(details, "88/tcp");
        if (ssh && kdc) return { ssh, kdc };
      } else if (details?.State?.Status === "exited") {
        throw containerFailure(container, "GSSAPI container exited during startup");
      }
    }
    await new Promise((resolveWait) => setTimeout(resolveWait, 50));
  }
  throw containerFailure(container, "timed out waiting for published GSSAPI ports");
}

function publishedPort(details, key) {
  const entry = details?.NetworkSettings?.Ports?.[key]?.[0]?.HostPort;
  const port = Number(entry);
  return Number.isInteger(port) && port > 0 && port <= 65535 ? port : null;
}

async function waitForTcp(port, container, label, expectSshBanner) {
  for (let attempt = 0; attempt < 200; attempt += 1) {
    const running = run("docker", ["inspect", "--format", "{{.State.Running}}", container], {
      capture: true,
      allowFailure: true,
    });
    if (running.status !== 0 || running.stdout.trim() !== "true") {
      throw containerFailure(container, `${label} container stopped during startup`);
    }
    if (await tcpReady(port, expectSshBanner)) return;
    await new Promise((resolveWait) => setTimeout(resolveWait, 50));
  }
  throw containerFailure(container, `timed out waiting for ${label} on 127.0.0.1:${port}`);
}

function tcpReady(port, expectSshBanner) {
  return new Promise((resolveReady) => {
    const socket = createConnection({ host: "127.0.0.1", port });
    let settled = false;
    let received = "";
    const finish = (ready) => {
      if (settled) return;
      settled = true;
      socket.destroy();
      resolveReady(ready);
    };
    socket.setTimeout(500);
    socket.once("connect", () => {
      if (!expectSshBanner) finish(true);
    });
    socket.on("data", (chunk) => {
      received += chunk.toString("ascii");
      if (received.split(/\r?\n/).some((line) => line.startsWith("SSH-"))) finish(true);
      if (received.length >= 1024) finish(false);
    });
    socket.once("timeout", () => finish(false));
    socket.once("error", () => finish(false));
    socket.once("close", () => finish(false));
  });
}

function containerFailure(container, message) {
  const logs = run("docker", ["logs", container], {
    capture: true,
    allowFailure: true,
  });
  return new Error(`${message}\n${logs.stdout}\n${logs.stderr}`.trim());
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
