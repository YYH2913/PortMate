import { describe, expect, it, vi } from "vitest";
import { findMcpPython } from "../tooling/python-runtime.mjs";

const options = { projectRoot: "/project with spaces", environment: { PATH: "/selected python" } };

describe("MCP Python discovery", () => {
  it("honors Windows PATH selection before trying the global launcher", () => {
    const run = vi.fn(() => ({ status: 0 }));
    expect(findMcpPython({ ...options, platform: "win32", run })).toEqual({ command: "python", args: [] });
    expect(run).toHaveBeenCalledExactlyOnceWith("python", [
      "-c", "import sys; raise SystemExit(sys.version_info < (3, 10))",
    ], {
      cwd: options.projectRoot, env: options.environment,
      encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], timeout: 10_000,
    });
  });

  it.each([
    { status: 1 },
    { status: null, error: Object.assign(new Error("missing"), { code: "ENOENT" }) },
    { status: null, signal: "SIGTERM" },
    { status: 0, signal: "SIGTERM" },
  ])("falls back to py -3 only when PATH Python is unusable: %j", result => {
    const run = vi.fn().mockReturnValueOnce(result).mockReturnValueOnce({ status: 0 });
    expect(findMcpPython({ ...options, platform: "win32", run })).toEqual({ command: "py", args: ["-3"] });
    expect(run.mock.calls.map(([command]) => command)).toEqual(["python", "py"]);
    expect(run.mock.calls[1][1].slice(0, 2)).toEqual(["-3", "-c"]);
  });

  it("keeps an explicit executable path byte-exact after trimming, with no implicit fallback", () => {
    const environment = { PORTMATE_PYTHON: " C:\\Python with spaces\\python.exe " };
    const run = vi.fn(() => ({ status: 0 }));
    expect(findMcpPython({ ...options, environment, platform: "win32", run }).command)
      .toBe("C:\\Python with spaces\\python.exe");
    const failure = vi.fn(() => ({ status: 1 }));
    expect(() => findMcpPython({ ...options, environment, platform: "win32", run: failure })).toThrow("PORTMATE_PYTHON");
    expect(failure).toHaveBeenCalledTimes(1);
  });

  it.each(["linux", "darwin"])("retains python3 priority on %s", platform => {
    const run = vi.fn(() => ({ status: 0 }));
    expect(findMcpPython({ ...options, platform, run })).toEqual({ command: "python3", args: [] });
  });

  it("rejects hosts without a supported interpreter", () => {
    const run = vi.fn(() => ({ status: 1 }));
    expect(() => findMcpPython({ ...options, platform: "win32", run })).toThrow("Python 3.10 or newer");
    expect(run).toHaveBeenCalledTimes(2);
  });
});
