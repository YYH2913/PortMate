import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const manifest = join(projectRoot, "scripts", "mcp-rust-client-check", "Cargo.toml");
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

const result = spawnSync("cargo", [
  "run",
  "--quiet",
  "--locked",
  "--manifest-path",
  manifest,
  "--",
  "--binary",
  binary,
], {
  cwd: projectRoot,
  env: {
    ...process.env,
    CARGO_TARGET_DIR: join(projectRoot, "target", "mcp-rust-sdk-check"),
  },
  encoding: "utf8",
  maxBuffer: 16 * 1024 * 1024,
});
if (result.stdout) process.stdout.write(result.stdout);
if (result.stderr) process.stderr.write(result.stderr);
if (result.error) throw result.error;
if (result.status !== 0) {
  throw new Error(`cargo run MCP Rust client check failed with exit code ${result.status}`);
}
