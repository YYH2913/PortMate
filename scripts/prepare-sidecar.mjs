import { copyFileSync, chmodSync, mkdirSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, isAbsolute, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));
const release = process.argv.includes("--release");
const profile = release ? "release" : "debug";
const hostTarget = rustHostTarget();
// Tauri's selected target must match the desktop binary, even when Cargo has
// a different default. An explicit host target also uses Cargo's triple layout.
const configuredTarget = process.env.TAURI_ENV_TARGET_TRIPLE
  || process.env.CARGO_BUILD_TARGET;
const target = configuredTarget || hostTarget;
const extension = target.includes("windows") ? ".exe" : "";
const cargoTargetDir = resolveTargetDirectory();
const source = configuredTarget
  ? resolve(cargoTargetDir, target, profile, `portmate-mcp${extension}`)
  : resolve(cargoTargetDir, profile, `portmate-mcp${extension}`);
const destination = resolve(
  projectRoot,
  "src-tauri",
  "binaries",
  `portmate-mcp-${target}${extension}`,
);

const args = ["build", "--locked", "-p", "portmate-mcp"];
if (configuredTarget) args.push("--target", target);
if (release) args.push("--release");
run("cargo", args);
mkdirSync(dirname(destination), { recursive: true });
copyFileSync(source, destination);
if (!extension) chmodSync(destination, 0o755);
console.log(`Prepared ${profile} MCP sidecar for ${target}: ${destination}`);

function rustHostTarget() {
  const result = spawnSync("rustc", ["-vV"], { cwd: projectRoot, encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error(`Unable to determine Rust host target:\n${result.stderr || result.stdout}`);
  }
  const host = result.stdout.split(/\r?\n/).find((line) => line.startsWith("host: "))?.slice(6).trim();
  if (!host) throw new Error("rustc -vV did not report a host target");
  return host;
}

function resolveTargetDirectory() {
  const configured = process.env.CARGO_TARGET_DIR;
  if (!configured) return resolve(projectRoot, "target");
  return isAbsolute(configured) ? configured : resolve(projectRoot, configured);
}

function run(command, args) {
  const result = spawnSync(command, args, { cwd: projectRoot, env: process.env, stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}
