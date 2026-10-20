import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, win32 } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { extractSdkToolArchive } from "../tooling/sdk-tool-archive.mjs";

const roots = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true, maxRetries: 3, retryDelay: 100 });
});

function workspace() {
  const root = mkdtempSync(join(tmpdir(), "sdk archive with spaces "));
  roots.push(root);
  const source = join(root, "source files");
  const destination = join(root, "installed SDK");
  mkdirSync(source);
  mkdirSync(destination);
  return { root, source, destination };
}

const windowsOptions = {
  archive: "D:\\SDK cache with spaces\\pinned tool.download",
  destination: "D:\\SDK cache with spaces\\installed tool",
  cwd: "D:\\project with spaces",
  platform: "win32",
  environment: { SystemRoot: "C:\\Windows", PATH: "C:\\Ruby\\msys64\\usr\\bin" },
};

describe("SDK tool archive extraction", () => {
  it.each([["tar.gz", "-xzf"], ["zip", "-xf"]])("uses Windows native tar for %s regardless of Ruby/MSYS2 PATH", (format, flag) => {
    const run = vi.fn(() => ({ status: 0 }));
    extractSdkToolArchive({ ...windowsOptions, format, timeout: 120_000, run });
    expect(run).toHaveBeenCalledExactlyOnceWith("C:\\Windows\\System32\\tar.exe", [
      flag, windowsOptions.archive, "-C", windowsOptions.destination,
    ], {
      cwd: windowsOptions.cwd, env: windowsOptions.environment,
      stdio: "inherit", shell: false, windowsHide: true, timeout: 120_000,
    });
  });

  it.each(["SystemRoot", "SYSTEMROOT", "systemroot"])("recognizes %s without assuming the system drive", name => {
    const run = vi.fn(() => ({ status: 0 }));
    extractSdkToolArchive({ ...windowsOptions, environment: { [name]: "E:\\Windows with spaces" }, run });
    expect(run.mock.calls[0][0]).toBe("E:\\Windows with spaces\\System32\\tar.exe");
  });

  it.each([undefined, "", "Windows", "C:Windows", "\\Windows", "C:\\Windows\0extra"])("refuses an ambiguous SystemRoot instead of falling back to PATH: %j", SystemRoot => {
    const run = vi.fn();
    expect(() => extractSdkToolArchive({ ...windowsOptions, environment: { SystemRoot }, run })).toThrow("SystemRoot");
    expect(run).not.toHaveBeenCalled();
  });

  it.each(["linux", "darwin", "freebsd"])("retains tar discovery and the original deadline on %s", platform => {
    const run = vi.fn(() => ({ status: 0 }));
    extractSdkToolArchive({ ...windowsOptions, platform, environment: {}, run });
    expect(run.mock.calls[0][0]).toBe("tar");
    expect(run.mock.calls[0][2].timeout).toBe(30_000);
  });

  it.each([{ status: 2 }, { status: null }, { status: null, signal: "SIGTERM" }, { status: 0, signal: "SIGTERM" }])("keeps extraction failure fatal: %j", result => {
    expect(() => extractSdkToolArchive({ ...windowsOptions, run: () => result })).toThrow("SDK archive extraction failed");
  });

  it.each(["ENOENT", "ETIMEDOUT"])("retains %s instead of retrying an unverified tool", code => {
    const error = Object.assign(new Error(code), { code });
    const run = vi.fn(() => ({ error }));
    expect(() => extractSdkToolArchive({ ...windowsOptions, run })).toThrow(error);
    expect(run).toHaveBeenCalledTimes(1);
  });

  it("rejects unsupported formats and unbounded deadlines", () => {
    const run = vi.fn();
    expect(() => extractSdkToolArchive({ ...windowsOptions, format: "exe", run })).toThrow("format");
    for (const timeout of [0, -1, Infinity, NaN, 0.5]) {
      expect(() => extractSdkToolArchive({ ...windowsOptions, timeout, run })).toThrow("timeout");
    }
    expect(run).not.toHaveBeenCalled();
  });

  it("extracts a real tar.gz with space-containing source and destination paths", () => {
    const { root, source, destination } = workspace();
    const payload = Buffer.from("pinned SDK payload\n");
    writeFileSync(join(source, "launcher.txt"), payload);
    const archive = join(root, "pinned tool.tar.gz");
    const command = process.platform === "win32"
      ? win32.join(process.env.SystemRoot, "System32", "tar.exe") : "tar";
    const created = spawnSync(command, ["-czf", archive, "-C", source, "."], { encoding: "utf8", timeout: 10_000 });
    expect(created.error ?? created.stderr).toBe("");
    expect(created.status).toBe(0);
    extractSdkToolArchive({ archive, destination, cwd: root });
    expect(readFileSync(join(destination, "launcher.txt"))).toEqual(payload);
  });

  it.runIf(process.platform === "win32")("extracts a real ZIP with Windows native tar", () => {
    const { root, destination } = workspace();
    const archive = join(root, "pinned tool.zip");
    // A stored, single-file ZIP containing sdk-tool/launcher.txt.
    writeFileSync(archive, Buffer.from("UEsDBBQAAAAAAAAAIQDhUQ8yEwAAABMAAAAVAAAAc2RrLXRvb2wvbGF1bmNoZXIudHh0cGlubmVkIFNESyBwYXlsb2FkClBLAQIUAxQAAAAAAAAAIQDhUQ8yEwAAABMAAAAVAAAAAAAAAAAAAACAAQAAAABzZGstdG9vbC9sYXVuY2hlci50eHRQSwUGAAAAAAEAAQBDAAAARgAAAAAA", "base64"));
    extractSdkToolArchive({ archive, destination, format: "zip", cwd: root });
    expect(readFileSync(join(destination, "sdk-tool", "launcher.txt"), "utf8")).toBe("pinned SDK payload\n");
  });

  it("rejects a corrupt archive using the real extraction tool", () => {
    const { root, destination } = workspace();
    const archive = join(root, "broken tool.tar.gz");
    writeFileSync(archive, "not an archive");
    const run = (command, args, options) => spawnSync(command, args, { ...options, stdio: "pipe" });
    expect(() => extractSdkToolArchive({ archive, destination, cwd: root, run })).toThrow("SDK archive extraction failed");
  });

  it.each(["mcp-jvm-tools.mjs", "mcp-csharp-client-check.mjs"])("routes the checksum-pinned %s bootstrap through the same extractor", name => {
    const source = readFileSync(new URL(`../tooling/${name}`, import.meta.url), "utf8");
    expect(source).toContain('import { extractSdkToolArchive } from "./sdk-tool-archive.mjs";');
    expect(source).toContain("extractSdkToolArchive({");
    expect(source).toContain("SHA-512 mismatch");
    expect(source).not.toContain('run("tar",');
  });
});
