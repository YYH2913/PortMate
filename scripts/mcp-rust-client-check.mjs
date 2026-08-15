import { cpSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const project = join(projectRoot, "scripts", "mcp-rust-client-check");
const matrix = JSON.parse(readFileSync(join(projectRoot, "scripts", "mcp-rust-client-versions.json"), "utf8"));
const configured = process.env.PORTMATE_MCP_BINARY?.trim();
const binary = configured || join(
  projectRoot,
  "target",
  "debug",
  process.platform === "win32" ? "portmate-mcp.exe" : "portmate-mcp",
);

if (!existsSync(binary)) {
  throw new Error(`MCP Rust client check binary does not exist: ${binary}`);
}

if (!Array.isArray(matrix) || !matrix.length || matrix.some((entry) => (
  typeof entry !== "object"
  || !/^\d+\.\d+\.\d+$/.test(entry.version)
  || !/^\d{4}-\d{2}-\d{2}$/.test(entry.protocolVersion)
))) {
  throw new Error("scripts/mcp-rust-client-versions.json must contain exact SDK and protocol versions");
}

for (const { version: sdkVersion, protocolVersion } of matrix) {
  const environmentRoot = join(projectRoot, "target", `mcp-rust-sdk-${sdkVersion}`);
  const sourceRoot = join(environmentRoot, "src");
  mkdirSync(sourceRoot, { recursive: true });
  cpSync(join(project, "src", "main.rs"), join(sourceRoot, "main.rs"));
  const manifest = `[package]
name = "portmate-mcp-rust-client-check-${sdkVersion.replaceAll(".", "-")}"
version = "0.0.0"
edition = "2024"
publish = false

[workspace]

[dependencies]
anyhow = "1.0"
rmcp = { version = "=${sdkVersion}", default-features = false, features = [
  "client",
  "transport-child-process",
  "transport-streamable-http-client-reqwest",
] }
serde_json = "1.0"
tokio = { version = "1.48", features = ["macros", "net", "process", "rt-multi-thread", "time"] }
`;
  const manifestPath = join(environmentRoot, "Cargo.toml");
  const manifestChanged = !existsSync(manifestPath) || readFileSync(manifestPath, "utf8") !== manifest;
  if (manifestChanged) {
    writeFileSync(manifestPath, manifest, "utf8");
  }

  const lockSource = join(project, "locks", sdkVersion, "Cargo.lock");
  if (!existsSync(lockSource)) {
    throw new Error(`MCP Rust SDK ${sdkVersion} lock file does not exist: ${lockSource}`);
  }
  const locked = readFileSync(lockSource, "utf8");
  if (!locked.includes(`name = "rmcp"\nversion = "${sdkVersion}"`)) {
    throw new Error(`MCP Rust SDK ${sdkVersion} lock file pins a different SDK version`);
  }
  cpSync(lockSource, join(environmentRoot, "Cargo.lock"));
  run("cargo", [
    "run",
    "--quiet",
    "--locked",
    "--manifest-path",
    manifestPath,
    "--",
    "--binary",
    binary,
  ], {
    ...process.env,
    CARGO_TARGET_DIR: join(environmentRoot, "target"),
    PORTMATE_MCP_RUST_SDK_VERSION: sdkVersion,
    PORTMATE_MCP_EXPECTED_PROTOCOL_VERSION: protocolVersion,
  });
}

function run(command, args, env) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    env,
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
    timeout: 600_000,
  });
  if (result.stdout) process.stdout.write(result.stdout);
  if (result.stderr) process.stderr.write(result.stderr);
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed with exit code ${result.status}`);
  }
}
