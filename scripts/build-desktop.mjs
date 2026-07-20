import { chmodSync, existsSync, lstatSync, readdirSync, rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
rmSync(join(projectRoot, "target", "release", "bundle"), { recursive: true, force: true });
if (process.platform !== "win32") {
  process.umask(0o022);
  chmodSync(join(projectRoot, "LICENSE"), 0o644);
  for (const entry of readdirSync(join(projectRoot, "src-tauri", "icons"))) {
    const path = join(projectRoot, "src-tauri", "icons", entry);
    if (lstatSync(path).isFile()) chmodSync(path, 0o644);
  }
  for (const binary of ["portmate", "portmate-mcp"]) {
    const path = join(projectRoot, "target", "release", binary);
    if (existsSync(path)) chmodSync(path, 0o755);
  }
}

run(process.execPath, [
  join(projectRoot, "node_modules", "@tauri-apps", "cli", "tauri.js"),
  "build",
  "--config",
  "src-tauri/tauri.bundle.conf.json",
]);
run(process.execPath, [join(projectRoot, "scripts", "finalize-linux-bundle.mjs")]);

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    env: process.env,
    stdio: "inherit",
  });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}
