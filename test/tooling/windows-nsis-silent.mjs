import { spawnSync } from "node:child_process";
import { win32 } from "node:path";

export function runSilentNsis({ executable, installationDirectory, phase, cwd, environment, run = spawnSync }) {
  const suffix = phase === "install" ? "/D=" : phase === "uninstall" ? "_?=" : undefined;
  if (!suffix) throw new Error(`Unsupported NSIS phase: ${phase}`);
  for (const path of [executable, installationDirectory]) {
    if (typeof path !== "string" || /[\x00-\x1f"]/.test(path)
        || !win32.isAbsolute(path) || win32.parse(path).root.length <= 1) {
      throw new Error("NSIS paths must be fully qualified Windows paths without quotes or control characters");
    }
  }
  // NSIS recognizes literal ` /D=` and ` _?=` and consumes the entire tail.
  // CRT-style quoting of these space-containing arguments hides the switches.
  // Quote only argv[0]; pass the directory tail verbatim without using a shell.
  const result = run(executable, ["/S", `${suffix}${installationDirectory}`], {
    argv0: `"${executable}"`,
    cwd,
    env: environment,
    stdio: "inherit",
    shell: false,
    windowsHide: true,
    windowsVerbatimArguments: true,
    timeout: 300_000,
  });
  if (result.error) throw result.error;
  if (result.signal || result.status !== 0) {
    throw new Error(`NSIS ${phase} failed: ${result.signal ?? `exit code ${result.status ?? 1}`}`);
  }
}
