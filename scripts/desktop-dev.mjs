import { spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { npmInvocation } from "./npm-invocation.mjs";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

console.log("Preparing the PortMate MCP sidecar before starting Tauri...");
const npm = npmInvocation(["run", "sidecar:dev"]);
run(npm.command, npm.args);
run(process.execPath, [
  join(projectRoot, "node_modules", "@tauri-apps", "cli", "tauri.js"),
  "dev",
  "--",
  "--locked",
]);

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    env: process.env,
    stdio: "inherit",
  });
  if (result.error) throw result.error;
  if (result.signal) {
    process.kill(process.pid, result.signal);
    return;
  }
  if (result.status !== 0) process.exit(result.status ?? 1);
}
