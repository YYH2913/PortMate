import { spawnSync } from "node:child_process";

// Only use for our built image after verifying the repository license payload.
// hdiutil asks for that embedded license on stdin, which is EOF in native CI.
export function mountVerifiedMacDmg({
  dmg,
  mountPoint,
  cwd,
  environment,
  run = spawnSync,
}) {
  const result = run("hdiutil", [
    "attach", dmg, "-readonly", "-nobrowse", "-mountpoint", mountPoint,
  ], {
    cwd,
    env: environment,
    input: "Y\n",
    stdio: ["pipe", "inherit", "inherit"],
    timeout: 120_000,
  });
  if (result.error) throw result.error;
  if (result.signal || result.status !== 0) {
    throw new Error(`hdiutil attach failed: ${result.signal ?? `exit code ${result.status ?? 1}`}`);
  }
}
