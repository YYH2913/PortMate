import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { buildDesktopEnvironment } from "../../scripts/desktop-clean-environment.mjs";
import { npmInvocation } from "../../scripts/npm-invocation.mjs";

const scriptPath = fileURLToPath(import.meta.url);
const projectRoot = resolve(dirname(scriptPath), "../..");

export function prebuildLinuxDesktop({
  environment = process.env,
  platform = process.platform,
  run = spawnSync,
} = {}) {
  if (platform !== "linux") throw new Error("The native Linux desktop prebuild requires Linux");
  // Match desktop:clean's environment so the prepared Cargo artifacts remain
  // usable when the timed development launch builds its application target.
  const env = buildDesktopEnvironment(environment, platform);
  const sidecar = npmInvocation(["run", "sidecar:dev"], { environment: env, platform });
  const commands = [sidecar, {
    command: "cargo",
    args: ["build", "--locked", "-p", "portmate", "--bin", "portmate"],
  }];
  for (const { command, args } of commands) {
    const result = run(command, args, {
      cwd: projectRoot,
      env,
      stdio: "inherit",
      timeout: 900_000,
    });
    if (result.error || result.signal || result.status !== 0) {
      throw new Error(`Linux desktop prebuild failed: ${command} (${result.error?.message ?? result.signal ?? result.status})`, {
        cause: result.error,
      });
    }
  }
}

if (process.argv[1] && resolve(process.argv[1]) === scriptPath) {
  try { prebuildLinuxDesktop(); }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}
