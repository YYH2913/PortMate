import { createHash } from "node:crypto";
import {
  createWriteStream,
  existsSync,
  mkdirSync,
  readFileSync,
  renameSync,
  rmSync,
} from "node:fs";
import { Readable, Transform } from "node:stream";
import { pipeline } from "node:stream/promises";
import { dirname, extname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const project = join(projectRoot, "scripts", "mcp-csharp-client-check", "McpCsharpClientCheck.csproj");
const matrix = JSON.parse(readFileSync(join(projectRoot, "scripts", "mcp-csharp-client-versions.json"), "utf8"));
validateMatrix(matrix);

const configured = process.env.PORTMATE_MCP_BINARY?.trim();
const binary = configured || join(
  projectRoot,
  "target",
  "debug",
  process.platform === "win32" ? "portmate-mcp.exe" : "portmate-mcp",
);
if (!existsSync(binary)) throw new Error(`MCP C# client check binary does not exist: ${binary}`);

const environment = {
  ...process.env,
  DOTNET_CLI_HOME: join(projectRoot, "target", "mcp-dotnet-home"),
  DOTNET_NOLOGO: "1",
  DOTNET_SKIP_FIRST_TIME_EXPERIENCE: "1",
  DOTNET_CLI_TELEMETRY_OPTOUT: "1",
  NUGET_PACKAGES: join(projectRoot, "target", "mcp-csharp-nuget-packages"),
};
mkdirSync(environment.DOTNET_CLI_HOME, { recursive: true });
mkdirSync(environment.NUGET_PACKAGES, { recursive: true });
const dotnet = await ensureDotnet(matrix.dotnet, environment);

for (const entry of matrix.sdks) {
  const properties = [
    `-p:McpSdkVersion=${entry.version}`,
    `-p:PortMateMcpProtocolVersion=${entry.protocolVersion}`,
  ];
  run(dotnet, [
    "restore",
    project,
    "--locked-mode",
    ...properties,
  ], { env: environment, timeout: 180_000 });
  run(dotnet, [
    "run",
    "--project",
    project,
    "--configuration",
    "Release",
    "--no-restore",
    ...properties,
    "--",
    resolve(binary),
    entry.version,
    entry.protocolVersion,
  ], { env: environment, timeout: 120_000 });
}

function validateMatrix(value) {
  const versionPattern = /^\d+\.\d+\.\d+$/;
  const archives = value?.dotnet?.archives;
  if (
    typeof value !== "object"
    || !versionPattern.test(value.dotnet?.version)
    || typeof archives !== "object"
    || !Array.isArray(value.sdks)
    || !value.sdks.length
    || value.sdks.some((entry) => (
      typeof entry !== "object"
      || !versionPattern.test(entry.version)
      || !/^\d{4}-\d{2}-\d{2}$/.test(entry.protocolVersion)
    ))
    || Object.values(archives).some((archive) => (
      typeof archive?.url !== "string"
      || !archive.url.startsWith("https://builds.dotnet.microsoft.com/")
      || !/^[a-f0-9]{128}$/.test(archive.sha512)
    ))
  ) {
    throw new Error("scripts/mcp-csharp-client-versions.json must pin exact SDK, protocol, and .NET archives");
  }
}

async function ensureDotnet({ version, archives }, environment) {
  const configuredDotnet = process.env.PORTMATE_DOTNET?.trim();
  const command = configuredDotnet || "dotnet";
  const probe = run(command, ["--version"], {
    env: environment,
    capture: true,
    allowFailure: true,
    timeout: 10_000,
  });
  if (probe.status === 0 && probe.stdout.trim() === version) return command;
  if (configuredDotnet) {
    throw new Error(`PORTMATE_DOTNET must point to .NET SDK ${version}; found ${probe.stdout.trim() || "unavailable"}`);
  }

  const platform = process.platform === "darwin" ? "osx" : process.platform === "win32" ? "win" : process.platform;
  const architecture = process.arch === "arm64" ? "arm64" : process.arch === "x64" ? "x64" : process.arch;
  const rid = `${platform}-${architecture}`;
  const archive = archives[rid];
  if (!archive) throw new Error(`No pinned .NET SDK ${version} archive for ${rid}`);

  const toolsRoot = join(projectRoot, "target", "mcp-dotnet-sdk-tools");
  const installRoot = join(toolsRoot, `dotnet-${version}-${rid}`);
  const executable = join(installRoot, process.platform === "win32" ? "dotnet.exe" : "dotnet");
  if (!existsSync(executable)) {
    mkdirSync(toolsRoot, { recursive: true });
    const archiveExtension = extname(new URL(archive.url).pathname) === ".zip" ? ".zip" : ".tar.gz";
    const archivePath = join(toolsRoot, `dotnet-sdk-${version}-${rid}${archiveExtension}`);
    const temporary = `${archivePath}.download`;
    const response = await fetch(archive.url, { signal: AbortSignal.timeout(300_000) });
    if (!response.ok || !response.body) {
      throw new Error(`Failed to download .NET SDK ${version} for ${rid}: HTTP ${response.status}`);
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
      throw new Error(`.NET SDK ${version} SHA-512 mismatch: expected ${archive.sha512}, found ${actual}`);
    }
    rmSync(installRoot, { recursive: true, force: true });
    mkdirSync(installRoot, { recursive: true });
    run("tar", [archiveExtension === ".zip" ? "-xf" : "-xzf", temporary, "-C", installRoot], {
      timeout: 120_000,
    });
    rmSync(archivePath, { force: true });
    renameSync(temporary, archivePath);
  }
  if (!existsSync(executable)) throw new Error(`.NET SDK archive omitted ${executable}`);
  environment.DOTNET_ROOT = installRoot;
  const installed = run(executable, ["--version"], { env: environment, capture: true, timeout: 10_000 });
  if (installed.stdout.trim() !== version) {
    throw new Error(`Expected .NET SDK ${version}, found ${installed.stdout.trim()}`);
  }
  return executable;
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
