import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync } from "node:fs";
import { dirname } from "node:path";

export function extractWindowsMsi({ installer, destination, logPath, diagnosticLogPath,
  cwd, environment, timeout = 300_000, run = spawnSync }) {
  if (!Number.isSafeInteger(timeout) || timeout <= 0) throw new Error("MSI extraction timeout must be a positive integer");
  const result = run("msiexec.exe", ["/a", installer, "/qn", "/norestart", "/l*v", logPath, `TARGETDIR=${destination}`], {
    cwd, env: environment, stdio: "inherit", shell: false, windowsHide: true, timeout,
  });
  let log = "";
  let logFailure;
  try {
    if (existsSync(logPath)) {
      const bytes = readFileSync(logPath);
      log = bytes.subarray(0, 2).equals(Buffer.from([0xff, 0xfe]))
        ? bytes.toString("utf16le").replace(/^\uFEFF/, "") : bytes.toString("utf8");
      mkdirSync(dirname(diagnosticLogPath), { recursive: true });
      copyFileSync(logPath, diagnosticLogPath);
    }
  } catch (error) {
    logFailure = error;
  }
  if (result.error || result.signal || result.status !== 0) {
    const reason = result.error?.message ?? result.signal ?? `exit code ${result.status ?? 1}`;
    const logDiagnostic = logFailure ? `\nMSI log retention failed: ${logFailure.message}` : "";
    throw new Error(`MSI administrative extraction failed: ${reason}${log ? `\n${log.slice(-32_768)}` : ""}${logDiagnostic}`, {
      cause: result.error,
    });
  }
  if (logFailure) throw logFailure;
}
