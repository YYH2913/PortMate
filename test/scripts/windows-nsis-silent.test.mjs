import { readFileSync } from "node:fs";
import { describe, expect, it, vi } from "vitest";
import { runSilentNsis } from "../tooling/windows-nsis-silent.mjs";

const packageCheck = readFileSync(new URL("../tooling/windows-package-check.mjs", import.meta.url), "utf8");

const options = {
  executable: "D:\\build with spaces\\PortMate installer.exe",
  installationDirectory: "D:\\audit & spaces\\installed app",
  cwd: "D:\\project",
  environment: { TEMP: "D:\\isolated temporary files" },
};

describe("NSIS silent package lifecycle", () => {
  it("uses the same raw-path helper for real silent install and uninstall checks", () => {
    expect(packageCheck).toContain('import { runSilentNsis } from "./windows-nsis-silent.mjs";');
    expect(packageCheck.match(/runSilentNsis\(/g)).toHaveLength(2);
    expect(packageCheck).not.toContain('run(nsis, ["/S", `/D=');
    expect(packageCheck).not.toContain('run(expectedUninstaller, ["/S", `_?=');
  });

  it.each([["install", "/D="], ["uninstall", "_?="]])("keeps the %s directory as NSIS's unquoted final tail", (phase, suffix) => {
    const run = vi.fn(() => ({ status: 0 }));
    runSilentNsis({ ...options, phase, run });
    expect(run).toHaveBeenCalledExactlyOnceWith(options.executable, ["/S", `${suffix}${options.installationDirectory}`], {
      argv0: `"${options.executable}"`,
      cwd: options.cwd,
      env: options.environment,
      stdio: "inherit",
      shell: false,
      windowsHide: true,
      windowsVerbatimArguments: true,
      timeout: 300_000,
    });
    // With verbatim arguments, libuv joins argv directly. NSIS searches for
    // these exact leading-space markers, not a CRT-decoded argv array.
    const [, args, spawnOptions] = run.mock.calls[0];
    const commandLine = [spawnOptions.argv0, ...args].join(" ");
    const marker = ` ${suffix}`;
    expect(commandLine.slice(commandLine.indexOf(marker) + marker.length)).toBe(options.installationDirectory);
    expect(commandLine).toBe(`"${options.executable}" /S ${suffix}${options.installationDirectory}`);
  });

  it.each([
    "relative\\app",
    "D:drive-relative\\app",
    "\\current-drive\\app",
    'D:\\quoted "app"',
    "D:\\app\nextra",
    "D:\\app\0extra",
  ])("refuses an ambiguous or unsafe raw directory: %j", installationDirectory => {
    const run = vi.fn();
    expect(() => runSilentNsis({ ...options, installationDirectory, phase: "install", run })).toThrow("fully qualified Windows paths");
    expect(run).not.toHaveBeenCalled();
  });

  it("refuses unsupported phases and ambiguous executable paths", () => {
    const run = vi.fn();
    expect(() => runSilentNsis({ ...options, phase: "repair", run })).toThrow("Unsupported NSIS phase");
    expect(() => runSilentNsis({ ...options, executable: "relative.exe", phase: "install", run })).toThrow("fully qualified Windows paths");
    expect(run).not.toHaveBeenCalled();
  });

  it.each([{ status: 1 }, { status: null, signal: "SIGTERM" }, { status: 0, signal: "SIGTERM" }])("retains failed installer status: %j", result => {
    expect(() => runSilentNsis({ ...options, phase: "install", run: () => result })).toThrow("NSIS install failed");
  });

  it("does not hide spawn errors or timeouts", () => {
    const error = Object.assign(new Error("installer timed out"), { code: "ETIMEDOUT" });
    expect(() => runSilentNsis({ ...options, phase: "uninstall", run: () => ({ error }) })).toThrow(error);
  });
});
