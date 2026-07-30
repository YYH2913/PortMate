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
if (!Array.isArray(matrix) || matrix.length < 5) {
  throw new Error("SSH GSSAPI server matrix must contain at least five entries");
}
const matrixNames = new Set();
for (const entry of matrix) {
  validateMatrixEntry(entry);
  if (matrixNames.has(entry.name)) {
    throw new Error(`duplicate SSH GSSAPI matrix name: ${entry.name}`);
  }
  matrixNames.add(entry.name);
}
const requestedServer = process.env.PORTMATE_COMPAT_GSSAPI_SERVER?.trim();
if (requestedServer && !matrixNames.has(requestedServer)) {
  throw new Error(`unknown SSH GSSAPI matrix server: ${requestedServer}`);
}
const selectedMatrix = requestedServer
  ? matrix.filter(({ name }) => name === requestedServer)
  : matrix;
const useCachedImages = compatibilityUsesCachedImages();
const temporaryRoot = mkdtempSync(join(tmpdir(), "portmate-gssapi-compat-"));
const verifiedServers = [];

try {
  run("docker", ["info", "--format", "{{.ServerVersion}}"], { quiet: true });
  run("bash", ["scripts/libssh-gssapi-build-check.sh"], {
    env: baseEnvironment,
    timeout: 600_000,
  });
  for (const entry of selectedMatrix) {
    const image = `portmate-compat-gssapi-${entry.name}:local`;
    const dockerfile = entry.dockerfile ?? "tests/compat/gssapi-openssh.Dockerfile";
    const buildArgs = entry.baseImage
      ? ["--build-arg", `PORTMATE_GSSAPI_BASE_IMAGE=${entry.baseImage}`]
      : [];
    await prepareCompatibilityImage({
      run,
      image,
      useCachedImages,
      buildArgs: [
        "build",
        "--tag",
        image,
        "--file",
        resolve(projectRoot, dockerfile),
        ...buildArgs,
        projectRoot,
      ],
      buildOptions: { timeout: 600_000 },
      inspectOptions: { timeout: controlTimeoutMs },
    });

    const cases = [];
    const versions = await withServer(entry, image, "yes", async (server) => {
      const kerberos = configureKerberos(server);
      acquireTicket(server, kerberos);
      runCase("success", server, kerberos, cases);
      runCase("gssapi-preferred", server, kerberos, cases);
      runCase("host-key-reject", server, kerberos, cases);
      corruptTicket(kerberos);
      runCase("corrupt-ticket", server, kerberos, cases);
      runCase("corrupt-ticket-password-fallback", server, kerberos, cases);
      destroyTicket(kerberos);
      runCase("no-ticket", server, kerberos, cases);
      runCase("password-fallback", server, kerberos, cases);
      return {
        ssh: server.version,
        kerberos: server.kerberosVersion,
      };
    });

    await withServer(entry, image, "no", async (server) => {
      const kerberos = configureKerberos(server);
      acquireTicket(server, kerberos);
      runCase("server-disabled", server, kerberos, cases);
      runCase("server-disabled-password-fallback", server, kerberos, cases);
      destroyTicket(kerberos);
    });
    for (const [mode, caseName] of [
      ["rejected", "sftp-rejected"],
      ["operation-denied", "sftp-operation-denied"],
    ]) {
      await withServer(entry, image, "yes", async (server) => {
        const kerberos = configureKerberos(server);
        acquireTicket(server, kerberos);
        runCase(caseName, server, kerberos, cases);
        destroyTicket(kerberos);
      }, mode);
    }
    verifiedServers.push({
      name: entry.name,
      implementation: entry.implementation ?? "openssh",
      kerberosImplementation: entry.kerberosImplementation ?? "mit",
      version: versions.ssh,
      ...(versions.kerberos ? { kerberosVersion: versions.kerberos } : {}),
      ...(entry.capAdd?.length ? {
        provisionCapabilities: entry.capAdd,
        droppedRuntimeCapabilities: entry.capAdd,
      } : {}),
      verifiedPtyResize: entry.verifyPtyResize !== false,
      cases,
    });
  }

  console.log(JSON.stringify({ verifiedGssapiServers: verifiedServers }, null, 2));
} finally {
  rmSync(temporaryRoot, { recursive: true, force: true });
}

async function withServer(entry, image, gssapiAuthentication, callback, sftpMode = "normal") {
  const container = `portmate-gssapi-${randomUUID()}`;
  const sshContainerPort = entry.sshContainerPort ?? 22;
  const capabilityArgs = (entry.capAdd ?? []).flatMap((capability) => [
    "--cap-add",
    capability,
  ]);
  try {
    run("docker", [
      "run",
      "--detach",
      "--name",
      container,
      "--hostname",
      "localhost",
      ...capabilityArgs,
      "--env",
      `PORTMATE_GSSAPI_AUTH=${gssapiAuthentication}`,
      "--env",
      `PORTMATE_GSSAPI_SFTP=${sftpMode}`,
      "--publish",
      `127.0.0.1::${sshContainerPort}/tcp`,
      "--publish",
      "127.0.0.1::88/tcp",
      image,
    ], { quiet: true, timeout: controlTimeoutMs });
    const ports = await waitForPublishedPorts(container, sshContainerPort);
    await waitForTcp(ports.kdc, container, "Kerberos KDC", false);
    await waitForTcp(ports.ssh, container, `${entry.implementation ?? "openssh"} SSH`, true);
    verifyDroppedRuntimeCapabilities(entry, container);
    const version = inspectServerVersion(entry, container);
    const kerberosVersion = inspectKerberosVersion(entry, container);
    try {
      return await callback({
        container,
        kerberosVersion,
        version,
        verifyPtyResize: entry.verifyPtyResize !== false,
        ...ports,
      });
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

function verifyDroppedRuntimeCapabilities(entry, container) {
  if (!(entry.capAdd ?? []).includes("SYS_ADMIN")) return;
  const result = run("docker", [
    "exec",
    container,
    "sh",
    "-lc",
    `for status in /proc/[0-9]*/status; do
      name="$(awk '$1 == "Name:" { print $2 }' "$status" 2>/dev/null || true)"
      case "$name" in
        samba|sshd)
          capability="$(awk '$1 == "CapBnd:" { print $2 }' "$status" 2>/dev/null || true)"
          if [ -n "$capability" ]; then
            printf '%s %s\\n' "$name" "$capability"
          fi
          ;;
      esac
    done`,
  ], {
    capture: true,
    timeout: controlTimeoutMs,
  });
  const capabilities = result.stdout.trim().split(/\r?\n/).filter(Boolean).map((line) => {
    const match = /^(samba|sshd) ([0-9a-f]+)$/i.exec(line);
    if (!match) {
      throw new Error(`invalid runtime capability output from ${container}: ${JSON.stringify(line)}`);
    }
    return { process: match[1], boundingSet: BigInt(`0x${match[2]}`) };
  });
  const processes = new Set(capabilities.map(({ process }) => process));
  if (!processes.has("samba") || !processes.has("sshd")) {
    throw new Error(`Samba and sshd capability state was not observable in ${container}`);
  }
  const sysAdmin = 1n << 21n;
  const retained = capabilities.find(({ boundingSet }) => (boundingSet & sysAdmin) !== 0n);
  if (retained) {
    throw new Error(`${retained.process} retained CAP_SYS_ADMIN in ${container}`);
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

function corruptTicket(kerberos) {
  writeFileSync(kerberos.cache, "PORTMATE_CORRUPT_KRB5_CCACHE\n", { mode: 0o600 });
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
      PORTMATE_COMPAT_GSSAPI_VERIFY_PTY_RESIZE: server.verifyPtyResize ? "1" : "0",
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
  const implementation = entry.implementation ?? "openssh";
  if (!new Set(["openssh", "apache-mina"]).has(implementation)) {
    throw new Error(`invalid SSH GSSAPI implementation for ${entry.name}`);
  }
  if (entry.baseImage !== undefined
    && (typeof entry.baseImage !== "string" || !/^[a-z0-9./:-]+$/.test(entry.baseImage))) {
    throw new Error(`invalid SSH GSSAPI base image for ${entry.name}`);
  }
  if (implementation === "openssh" && typeof entry.baseImage !== "string") {
    throw new Error(`OpenSSH GSSAPI entry requires a base image for ${entry.name}`);
  }
  if (entry.dockerfile !== undefined
    && (typeof entry.dockerfile !== "string" || !entry.dockerfile.startsWith("tests/compat/"))) {
    throw new Error(`invalid SSH GSSAPI Dockerfile for ${entry.name}`);
  }
  if (entry.sshContainerPort !== undefined
    && (!Number.isInteger(entry.sshContainerPort)
      || entry.sshContainerPort <= 0
      || entry.sshContainerPort > 65535)) {
    throw new Error(`invalid SSH GSSAPI container port for ${entry.name}`);
  }
  if (entry.verifyPtyResize !== undefined && typeof entry.verifyPtyResize !== "boolean") {
    throw new Error(`invalid SSH GSSAPI PTY resize flag for ${entry.name}`);
  }
  const kerberosImplementation = entry.kerberosImplementation ?? "mit";
  if (!new Set(["mit", "samba-ad-compatible"]).has(kerberosImplementation)) {
    throw new Error(`invalid Kerberos implementation for ${entry.name}`);
  }
  const capabilities = entry.capAdd ?? [];
  if (!Array.isArray(capabilities)
    || capabilities.some((capability) => typeof capability !== "string")
    || new Set(capabilities).size !== capabilities.length) {
    throw new Error(`invalid Docker capability list for ${entry.name}`);
  }
  const requiredCapabilities = kerberosImplementation === "samba-ad-compatible"
    ? ["SYS_ADMIN"]
    : [];
  if (capabilities.length !== requiredCapabilities.length
    || capabilities.some((capability, index) => capability !== requiredCapabilities[index])) {
    throw new Error(`unexpected Docker capabilities for ${entry.name}`);
  }
  if (kerberosImplementation === "samba-ad-compatible"
    && entry.dockerfile !== "tests/compat/gssapi-samba-ad.Dockerfile") {
    throw new Error(`Samba AD-compatible entry requires its controlled Dockerfile for ${entry.name}`);
  }
  if (entry.kerberosVersionPattern !== undefined) {
    if (typeof entry.kerberosVersionPattern !== "string") {
      throw new Error(`invalid Kerberos version pattern for ${entry.name}`);
    }
    new RegExp(entry.kerberosVersionPattern);
  }
  if (kerberosImplementation === "samba-ad-compatible"
    && typeof entry.kerberosVersionPattern !== "string") {
    throw new Error(`Samba AD-compatible entry requires a version pattern for ${entry.name}`);
  }
  if (entry.versionPattern !== undefined) {
    if (typeof entry.versionPattern !== "string") {
      throw new Error(`invalid SSH GSSAPI version pattern for ${entry.name}`);
    }
    new RegExp(entry.versionPattern);
  }
}

function inspectKerberosVersion(entry, container) {
  const implementation = entry.kerberosImplementation ?? "mit";
  if (implementation === "mit") return null;
  const versionResult = run("docker", ["exec", container, "samba", "--version"], {
    capture: true,
    timeout: controlTimeoutMs,
  });
  const version = `${versionResult.stdout}${versionResult.stderr}`.trim();
  const pattern = new RegExp(entry.kerberosVersionPattern);
  if (!pattern.test(version)) {
    throw new Error(`invalid ${implementation} version from ${container}: ${JSON.stringify(version)}`);
  }
  return version;
}

function inspectServerVersion(entry, container) {
  const implementation = entry.implementation ?? "openssh";
  const command = implementation === "apache-mina"
    ? ["exec", container, "java", "-jar", "/opt/portmate/apache-mina-gssapi-server.jar", "--version"]
    : ["exec", container, "/usr/sbin/sshd", "-V"];
  const versionResult = run("docker", command, {
    capture: true,
    timeout: controlTimeoutMs,
  });
  const version = `${versionResult.stdout}${versionResult.stderr}`.trim();
  const pattern = new RegExp(entry.versionPattern ?? "^OpenSSH_[0-9]");
  if (!pattern.test(version)) {
    throw new Error(`invalid ${implementation} version from ${container}: ${JSON.stringify(version)}`);
  }
  return version;
}

async function waitForPublishedPorts(container, sshContainerPort) {
  for (let attempt = 0; attempt < 200; attempt += 1) {
    const inspected = run("docker", ["inspect", container], {
      capture: true,
      allowFailure: true,
      timeout: controlTimeoutMs,
    });
    if (inspected.status === 0) {
      const details = JSON.parse(inspected.stdout)[0];
      if (details?.State?.Running) {
        const ssh = publishedPort(details, `${sshContainerPort}/tcp`);
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
