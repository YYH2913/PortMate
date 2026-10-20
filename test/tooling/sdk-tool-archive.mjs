import { spawnSync } from "node:child_process";
import { win32 } from "node:path";

export function extractSdkToolArchive({
  archive,
  destination,
  format = "tar.gz",
  cwd,
  environment = process.env,
  platform = process.platform,
  timeout = 30_000,
  run = spawnSync,
}) {
  if (!["tar.gz", "zip"].includes(format)) throw new Error(`Unsupported SDK archive format: ${format}`);
  if (!Number.isSafeInteger(timeout) || timeout <= 0) throw new Error("SDK extraction timeout must be a positive integer");
  let command = "tar";
  if (platform === "win32") {
    const systemRoot = Object.entries(environment)
      .find(([name]) => name.toLowerCase() === "systemroot")?.[1];
    if (typeof systemRoot !== "string" || /[\x00-\x1f"]/.test(systemRoot)
        || !win32.isAbsolute(systemRoot) || win32.parse(systemRoot).root.length <= 1) {
      throw new Error("Windows SDK archive extraction requires a fully qualified SystemRoot");
    }
    // Ruby/MSYS2 can put GNU tar first on PATH. It treats D:\\... as a
    // remote archive and cannot extract ZIPs. Windows' native bsdtar handles both.
    command = win32.join(systemRoot, "System32", "tar.exe");
  }
  const result = run(command, [format === "zip" ? "-xf" : "-xzf", archive, "-C", destination], {
    cwd,
    env: environment,
    stdio: "inherit",
    shell: false,
    windowsHide: true,
    timeout,
  });
  if (result.error) throw result.error;
  if (result.signal || result.status !== 0) {
    throw new Error(`SDK archive extraction failed: ${result.signal ?? `exit code ${result.status ?? 1}`}`);
  }
}
