import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { extractWindowsMsi } from "../tooling/windows-msi-admin-extract.mjs";

const roots = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true, maxRetries: 3, retryDelay: 100 });
});
function options() {
  const root = mkdtempSync(join(tmpdir(), "msi check with spaces "));
  roots.push(root);
  return {
    installer: join(root, "PortMate installer.msi"), destination: join(root, "installed app"),
    logPath: join(root, "msi-extract.log"), diagnosticLogPath: join(root, "diagnostics", "windows-msi-extract.log"),
    cwd: root, environment: process.env,
  };
}

describe("bounded MSI administrative extraction", () => {
  it("quotes only the property value using MSI syntax and retains a finite install budget", () => {
    const input = options();
    const run = vi.fn(() => ({ status: 0 }));
    extractWindowsMsi({ ...input, run });
    expect(run).toHaveBeenCalledExactlyOnceWith("msiexec.exe", [
      "/a", `"${input.installer}"`, "/qn", "/norestart", "/l*v", `"${input.logPath}"`, `TARGETDIR="${input.destination}"`,
    ], { cwd: input.cwd, env: input.environment, stdio: "inherit", shell: false, windowsHide: true, windowsVerbatimArguments: true, timeout: 300_000 });
    const commandLine = ["msiexec.exe", ...run.mock.calls[0][1]].join(" ");
    expect(commandLine).toContain(` TARGETDIR="${input.destination}"`);
    expect(commandLine).not.toContain(' "TARGETDIR=');
  });

  it.each(["installer", "destination", "logPath"])("refuses quotes and control characters in raw %s", field => {
    for (const value of ['D:\\quoted "path"', "D:\\path\nextra", "D:\\path\0extra", ""]) {
      const run = vi.fn();
      expect(() => extractWindowsMsi({ ...options(), [field]: value, run })).toThrow("MSI paths");
      expect(run).not.toHaveBeenCalled();
    }
  });

  it.each(["utf8", "utf16le"])("retains the original %s installer log before workspace cleanup", encoding => {
    const input = options();
    const log = Buffer.from(`${encoding === "utf16le" ? "\uFEFF" : ""}MSI error 1603: administrative extraction failed\r\n`, encoding);
    writeFileSync(input.logPath, log);
    expect(() => extractWindowsMsi({ ...input, run: () => ({ status: 1603 }) })).toThrow("MSI error 1603");
    rmSync(input.logPath);
    expect(readFileSync(input.diagnosticLogPath)).toEqual(log);
  });

  it("keeps successful installer logs for later package failures", () => {
    const input = options();
    writeFileSync(input.logPath, "MSI administrative installation completed");
    extractWindowsMsi({ ...input, run: () => ({ status: 0 }) });
    expect(readFileSync(input.diagnosticLogPath, "utf8")).toContain("installation completed");
  });

  it("does not hide an MSI failure when retaining its log also fails", () => {
    const input = options();
    writeFileSync(input.logPath, "original installer failure");
    writeFileSync(join(input.cwd, "diagnostics"), "blocking file");
    let failure;
    try { extractWindowsMsi({ ...input, run: () => ({ status: 1603 }) }); } catch (error) { failure = error; }
    expect(failure.message).toContain("exit code 1603");
    expect(failure.message).toContain("original installer failure");
    expect(failure.message).toContain("MSI log retention failed");
  });

  it("terminates a stalled real child and retains its partial installer log", () => {
    const input = options();
    const run = (_command, _args, settings) => spawnSync(process.execPath, [
      "-e", "require('node:fs').writeFileSync(process.argv[1], 'MSI phase entered\\n'); setInterval(() => {}, 1000)", input.logPath,
    ], { ...settings, windowsVerbatimArguments: false });
    expect(() => extractWindowsMsi({ ...input, timeout: 2000, run })).toThrow("MSI phase entered");
    expect(readFileSync(input.diagnosticLogPath, "utf8")).toBe("MSI phase entered\n");
  });

  it.each([{ status: null }, { status: 0, signal: "SIGTERM" }, { error: new Error("installer missing") }])("refuses incomplete or failed installer execution: %j", result => {
    expect(() => extractWindowsMsi({ ...options(), run: () => result })).toThrow("MSI administrative extraction failed");
  });

  it("bounds the error excerpt while retaining the complete diagnostic log", () => {
    const input = options();
    const log = `${"x".repeat(80_000)}\nfinal MSI error`;
    writeFileSync(input.logPath, log);
    let failure;
    try { extractWindowsMsi({ ...input, run: () => ({ status: 1603 }) }); } catch (error) { failure = error; }
    expect(failure.message).toContain("final MSI error");
    expect(failure.message.length).toBeLessThan(33_000);
    expect(readFileSync(input.diagnosticLogPath, "utf8")).toBe(log);
  });

  it.each([0, -1, Infinity, NaN])("refuses an unbounded or invalid deadline: %j", timeout => {
    const run = vi.fn();
    expect(() => extractWindowsMsi({ ...options(), timeout, run })).toThrow("timeout");
    expect(run).not.toHaveBeenCalled();
  });
});
