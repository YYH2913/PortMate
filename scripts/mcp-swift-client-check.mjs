import { createHash } from "node:crypto";
import { createServer } from "node:net";
import {
  cpSync,
  createReadStream,
  createWriteStream,
  existsSync,
  mkdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, delimiter, join, resolve } from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { Readable, Transform } from "node:stream";
import { pipeline } from "node:stream/promises";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const templateRoot = join(projectRoot, "scripts", "mcp-swift-client-check");
const matrix = JSON.parse(readFileSync(join(projectRoot, "scripts", "mcp-swift-client-versions.json"), "utf8"));
validateMatrix(matrix);
if (process.platform === "win32") {
  throw new Error(
    "The selected official Swift SDK versions cannot build on Windows: their HTTP source imports EventSource, but the packages declare that dependency only for Apple platforms",
  );
}

const configured = process.env.PORTMATE_MCP_BINARY?.trim();
const binary = configured || join(
  projectRoot,
  "target",
  "debug",
  process.platform === "win32" ? "portmate-mcp.exe" : "portmate-mcp",
);
if (!existsSync(binary)) throw new Error(`MCP Swift client check binary does not exist: ${binary}`);

const environment = { ...process.env };
const swift = await ensureSwift(matrix.swift, environment);
const cache = join(projectRoot, "target", "mcp-swift-cache");
mkdirSync(cache, { recursive: true });

for (const entry of matrix.sdks) {
  const environmentRoot = join(projectRoot, "target", `mcp-swift-sdk-${entry.version}`);
  const sourceRoot = join(environmentRoot, "Sources", "McpSwiftClientCheck");
  const scratch = join(environmentRoot, "build");
  mkdirSync(sourceRoot, { recursive: true });
  copyIfChanged(
    join(templateRoot, "Sources", "McpSwiftClientCheck", "main.swift"),
    join(sourceRoot, "main.swift"),
  );
  const manifest = swiftManifest(entry.version);
  const manifestPath = join(environmentRoot, "Package.swift");
  const manifestChanged = !existsSync(manifestPath) || readFileSync(manifestPath, "utf8") !== manifest;
  if (manifestChanged) writeFileSync(manifestPath, manifest, "utf8");
  const lockSource = join(templateRoot, "locks", entry.version, "Package.resolved");
  if (!existsSync(lockSource)) {
    throw new Error(`Swift ${entry.version} lock file does not exist: ${lockSource}`);
  }
  const resolved = JSON.parse(readFileSync(lockSource, "utf8"));
  const sdkPin = resolved.pins?.find((pin) => pin.identity === "swift-sdk");
  if (sdkPin?.state?.version !== entry.version) {
    throw new Error(`Swift ${entry.version} lock pins ${sdkPin?.state?.version ?? "no SDK version"}`);
  }
  copyIfChanged(lockSource, join(environmentRoot, "Package.resolved"));
  const buildArguments = [
    "build",
    "--package-path",
    environmentRoot,
    "--cache-path",
    cache,
    "--scratch-path",
    scratch,
    "--disable-automatic-resolution",
  ];
  buildArguments.push(
    "--configuration",
    "release",
    "--product",
    "McpSwiftClientCheck",
  );
  run(swift, buildArguments, { env: environment, timeout: 600_000 });

  const client = join(
    scratch,
    "release",
    process.platform === "win32" ? "McpSwiftClientCheck.exe" : "McpSwiftClientCheck",
  );
  if (!existsSync(client)) throw new Error(`Swift ${entry.version} build omitted ${client}`);

  run(client, ["stdio", resolve(binary), entry.version, entry.protocolVersion], {
    env: environment,
    timeout: 120_000,
  });

  const token = "portmate-mcp-swift-http-client-check";
  const port = await reservePort();
  const endpoint = `http://127.0.0.1:${port}/mcp`;
  const bridge = startHttpBridge(binary, port, token);
  try {
    await waitForHttp(endpoint, bridge);
    run(client, ["http", endpoint, entry.version, entry.protocolVersion, token], {
      env: environment,
      timeout: 120_000,
    });
  } finally {
    await stopProcess(bridge);
  }
}

function swiftManifest(sdkVersion) {
  return `// swift-tools-version: 6.1

import PackageDescription

let package = Package(
    name: "mcp-swift-client-check",
    platforms: [
        .macOS(.v13),
    ],
    products: [
        .executable(name: "McpSwiftClientCheck", targets: ["McpSwiftClientCheck"]),
    ],
    dependencies: [
        .package(url: "https://github.com/modelcontextprotocol/swift-sdk.git", exact: "${sdkVersion}"),
        .package(url: "https://github.com/apple/swift-system.git", exact: "1.4.0"),
    ],
    targets: [
        .executableTarget(
            name: "McpSwiftClientCheck",
            dependencies: [
                .product(name: "MCP", package: "swift-sdk"),
                .product(name: "SystemPackage", package: "swift-system"),
            ]
        ),
    ]
)
`;
}

function copyIfChanged(source, destination) {
  const sourceBytes = readFileSync(source);
  if (existsSync(destination) && readFileSync(destination).equals(sourceBytes)) return;
  cpSync(source, destination);
}

function validateMatrix(value) {
  const versionPattern = /^\d+\.\d+\.\d+$/;
  const archives = value?.swift?.archives;
  if (
    typeof value !== "object"
    || !versionPattern.test(value?.swift?.version)
    || typeof archives !== "object"
    || !Object.keys(archives).length
    || Object.values(archives).some((archive) => (
      typeof archive?.url !== "string"
      || !archive.url.startsWith("https://download.swift.org/")
      || !/^[a-f0-9]{128}$/.test(archive.sha512)
    ))
    || !Array.isArray(value.sdks)
    || !value.sdks.length
    || value.sdks.some((entry) => (
      typeof entry !== "object"
      || !versionPattern.test(entry.version)
      || !/^\d{4}-\d{2}-\d{2}$/.test(entry.protocolVersion)
    ))
  ) {
    throw new Error("scripts/mcp-swift-client-versions.json must pin exact Swift, SDK, protocol, and archive versions");
  }
}

async function ensureSwift({ version, archives }, environment) {
  const configuredSwift = process.env.PORTMATE_SWIFT?.trim();
  const command = configuredSwift || "swift";
  const probe = run(command, ["--version"], {
    env: environment,
    capture: true,
    allowFailure: true,
    timeout: 10_000,
  });
  if (probe.status === 0 && matchesSwiftVersion(probe.stdout, version)) {
    return command;
  }
  if (configuredSwift) {
    throw new Error(`PORTMATE_SWIFT must point to Swift ${version}; found ${probe.stdout.trim() || "unavailable"}`);
  }

  const platform = process.platform === "linux" ? "linux" : process.platform;
  const architecture = process.arch === "x64" ? "x64" : process.arch;
  const rid = `${platform}-${architecture}`;
  const archive = archives[rid];
  if (!archive) {
    throw new Error(`Swift ${version} is required; no self-contained bootstrap archive is pinned for ${rid}`);
  }

  const toolsRoot = join(projectRoot, "target", "mcp-swift-tools");
  const installRoot = join(toolsRoot, `swift-${version}-${rid}`);
  const executable = join(installRoot, "usr", "bin", "swift");
  const archivePath = join(projectRoot, "target", `swift-${version}-RELEASE-ubuntu24.04.tar.gz`);
  if (!existsSync(executable)) {
    mkdirSync(toolsRoot, { recursive: true });
    if (!existsSync(archivePath)) {
      const temporary = `${archivePath}.download`;
      const response = await fetch(archive.url, { signal: AbortSignal.timeout(600_000) });
      if (!response.ok || !response.body) {
        throw new Error(`Failed to download Swift ${version} for ${rid}: HTTP ${response.status}`);
      }
      const digest = createHash("sha512");
      await pipeline(
        Readable.fromWeb(response.body),
        new Transform({
          transform(chunk, encoding, callback) {
            digest.update(chunk);
            callback(null, chunk);
          },
        }),
        createWriteStream(temporary, { mode: 0o600 }),
      );
      const actual = digest.digest("hex");
      if (actual !== archive.sha512) {
        rmSync(temporary, { force: true });
        throw new Error(`Swift ${version} SHA-512 mismatch: expected ${archive.sha512}, found ${actual}`);
      }
      renameSync(temporary, archivePath);
    } else {
      const actual = await hashFile(archivePath, "sha512");
      if (actual !== archive.sha512) {
        throw new Error(`Cached Swift ${version} SHA-512 mismatch: expected ${archive.sha512}, found ${actual}`);
      }
    }
    const temporaryInstall = `${installRoot}.extract`;
    rmSync(temporaryInstall, { recursive: true, force: true });
    mkdirSync(temporaryInstall, { recursive: true });
    run("tar", ["-xzf", archivePath, "--strip-components=1", "-C", temporaryInstall], {
      timeout: 180_000,
    });
    const temporaryExecutable = join(temporaryInstall, "usr", "bin", "swift");
    if (!existsSync(temporaryExecutable)) {
      throw new Error(`Swift archive omitted ${temporaryExecutable}`);
    }
    rmSync(installRoot, { recursive: true, force: true });
    renameSync(temporaryInstall, installRoot);
  }
  if (!existsSync(executable)) throw new Error(`Swift archive omitted ${executable}`);
  environment.PATH = `${dirname(executable)}${delimiter}${environment.PATH ?? ""}`;
  const installed = run(executable, ["--version"], { env: environment, capture: true, timeout: 10_000 });
  if (!matchesSwiftVersion(installed.stdout, version)) {
    throw new Error(`Expected Swift ${version}, found ${installed.stdout.trim()}`);
  }
  return executable;
}

function matchesSwiftVersion(output, version) {
  return output.startsWith(`Swift version ${version} `)
    || output.startsWith(`Apple Swift version ${version} `);
}

async function hashFile(path, algorithm) {
  const hash = createHash(algorithm);
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest("hex");
}

function reservePort() {
  return new Promise((resolvePort, reject) => {
    const server = createServer();
    server.unref();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      const port = typeof address === "object" && address ? address.port : 0;
      server.close((error) => error ? reject(error) : resolvePort(port));
    });
  });
}

function startHttpBridge(path, port, token) {
  const child = spawn(resolve(path), ["--http"], {
    cwd: projectRoot,
    env: {
      ...process.env,
      PORTMATE_MCP_HTTP_ADDR: `127.0.0.1:${port}`,
      PORTMATE_MCP_HTTP_TOKEN: token,
      PORTMATE_MCP_CLIENT_ID: "official-swift-sdk-http-check",
      PORTMATE_STORE_PATH: "",
    },
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  child.output = "";
  for (const stream of [child.stdout, child.stderr]) {
    stream.on("data", (chunk) => {
      child.output = `${child.output}${chunk}`.slice(-64 * 1024);
    });
  }
  return child;
}

async function waitForHttp(endpoint, child) {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    if (child.exitCode !== null) {
      throw new Error(`PortMate HTTP bridge exited during startup${child.output.trim() ? `\n${child.output.trim()}` : ""}`);
    }
    try {
      const response = await fetch(endpoint, { method: "OPTIONS", signal: AbortSignal.timeout(250) });
      if (response.status === 204) return;
    } catch {
      // The listener may not be ready yet.
    }
    await new Promise((resolveDelay) => setTimeout(resolveDelay, 50));
  }
  throw new Error(`Timed out waiting for ${endpoint}`);
}

async function stopProcess(child) {
  if (child.exitCode !== null) return;
  child.kill();
  await Promise.race([
    new Promise((resolveExit) => child.once("exit", resolveExit)),
    new Promise((resolveDelay) => setTimeout(resolveDelay, 2_000)),
  ]);
  if (child.exitCode === null) child.kill("SIGKILL");
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    env: options.env ?? process.env,
    encoding: "utf8",
    stdio: options.capture ? ["ignore", "pipe", "pipe"] : "inherit",
    maxBuffer: 32 * 1024 * 1024,
    timeout: options.timeout ?? 30_000,
  });
  if (result.error && !options.allowFailure) {
    if (result.error.code === "ETIMEDOUT") {
      throw new Error(`${command} exceeded its ${options.timeout ?? 30_000} ms timeout`);
    }
    throw result.error;
  }
  if (result.status !== 0 && !options.allowFailure) {
    const details = [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
    throw new Error(`${command} failed with exit code ${result.status ?? 1}${details ? `\n${details}` : ""}`);
  }
  return result;
}
