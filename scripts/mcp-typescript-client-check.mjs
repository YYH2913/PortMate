import { cpSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { npmInvocation } from "./npm-invocation.mjs";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const matrix = JSON.parse(readFileSync(
  join(projectRoot, "scripts", "mcp-typescript-client-versions.json"),
  "utf8",
));
if (!Array.isArray(matrix) || !matrix.length || matrix.some((entry) => (
  typeof entry !== "object"
  || !/^\d+\.\d+\.\d+$/.test(entry.version)
  || !/^\d{4}-\d{2}-\d{2}$/.test(entry.requestedProtocolVersion)
  || !/^\d{4}-\d{2}-\d{2}$/.test(entry.protocolVersion)
  || typeof entry.protocolHeader !== "boolean"
))) {
  throw new Error("scripts/mcp-typescript-client-versions.json must contain exact SDK and protocol versions");
}

const configured = process.env.PORTMATE_MCP_BINARY?.trim();
const binary = configured || join(
  projectRoot,
  "target",
  "debug",
  process.platform === "win32" ? "portmate-mcp.exe" : "portmate-mcp",
);
if (!existsSync(binary)) {
  throw new Error(`MCP TypeScript client check binary does not exist: ${binary}`);
}

for (const {
  version: sdkVersion,
  requestedProtocolVersion,
  protocolVersion,
  protocolHeader,
} of matrix) {
  const environmentRoot = join(projectRoot, "target", `mcp-typescript-sdk-${sdkVersion}`);
  mkdirSync(environmentRoot, { recursive: true });
  const manifest = `${JSON.stringify({
    name: `portmate-mcp-typescript-client-check-${sdkVersion.replaceAll(".", "-")}`,
    private: true,
    type: "module",
    dependencies: { "@modelcontextprotocol/sdk": sdkVersion },
  }, null, 2)}\n`;
  const manifestPath = join(environmentRoot, "package.json");
  const manifestChanged = !existsSync(manifestPath) || readFileSync(manifestPath, "utf8") !== manifest;
  if (manifestChanged) writeFileSync(manifestPath, manifest, "utf8");

  const lockPath = join(environmentRoot, "package-lock.json");
  const lockSource = join(
    projectRoot,
    "scripts",
    "mcp-typescript-client-check",
    "locks",
    sdkVersion,
    "package-lock.json",
  );
  if (!existsSync(lockSource)) {
    throw new Error(`MCP TypeScript SDK ${sdkVersion} lock file does not exist: ${lockSource}`);
  }
  const lock = JSON.parse(readFileSync(lockSource, "utf8"));
  if (lock.packages?.[""]?.dependencies?.["@modelcontextprotocol/sdk"] !== sdkVersion) {
    throw new Error(`MCP TypeScript SDK ${sdkVersion} lock file pins a different SDK version`);
  }
  cpSync(lockSource, lockPath);
  const npm = npmInvocation([
    "ci",
    "--ignore-scripts",
    "--no-audit",
    "--no-fund",
  ]);
  run(npm.command, npm.args, { cwd: environmentRoot, timeout: 120_000 });

  const sdkRoot = join(environmentRoot, "node_modules", "@modelcontextprotocol", "sdk");
  const installed = JSON.parse(readFileSync(join(sdkRoot, "package.json"), "utf8"));
  if (installed.version !== sdkVersion) {
    throw new Error(`Expected TypeScript SDK ${sdkVersion}, found ${installed.version ?? "unknown"}`);
  }
  const environment = {
    ...process.env,
    PORTMATE_MCP_BINARY: binary,
    PORTMATE_MCP_EXPECTED_PROTOCOL_VERSION: protocolVersion,
    PORTMATE_MCP_EXPECTED_REQUEST_PROTOCOL_VERSION: requestedProtocolVersion,
    PORTMATE_MCP_TYPESCRIPT_SDK_ROOT: sdkRoot,
    PORTMATE_MCP_TYPESCRIPT_SDK_VERSION: sdkVersion,
    PORTMATE_MCP_TYPESCRIPT_EXPECT_PROTOCOL_HEADER: protocolHeader ? "1" : "0",
  };
  for (const script of ["mcp-stdio-client-check.mjs", "mcp-http-client-check.mjs"]) {
    run(process.execPath, [join(projectRoot, "scripts", script)], {
      cwd: projectRoot,
      env: environment,
      timeout: 90_000,
    });
  }
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd ?? projectRoot,
    env: options.env ?? process.env,
    encoding: "utf8",
    stdio: "inherit",
    timeout: options.timeout ?? 30_000,
  });
  if (result.error) {
    if (result.error.code === "ETIMEDOUT") {
      throw new Error(`${command} exceeded its ${options.timeout ?? 30_000} ms timeout`);
    }
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed with exit code ${result.status}`);
  }
}
