import { describe, expect, it } from "vitest";
import { npmInvocation } from "./npm-invocation.mjs";

describe("npm process invocation", () => {
  it("runs the npm JavaScript entrypoint directly on Windows", () => {
    expect(npmInvocation(["audit", "--json"], {
      environment: { npm_execpath: "C:\\node\\node_modules\\npm\\bin\\npm-cli.js" },
      execPath: "C:\\node\\node.exe",
      platform: "win32",
      pathExists: () => true,
    })).toEqual({
      command: "C:\\node\\node.exe",
      args: ["C:\\node\\node_modules\\npm\\bin\\npm-cli.js", "audit", "--json"],
    });
  });

  it("uses the Node-adjacent npm CLI when npm_execpath is unavailable", () => {
    expect(npmInvocation(["run", "build"], {
      environment: {},
      execPath: "C:\\node\\node.exe",
      platform: "win32",
      pathExists: (path) => path.endsWith("node_modules\\npm\\bin\\npm-cli.js"),
    }).args).toEqual([
      "C:\\node\\node_modules\\npm\\bin\\npm-cli.js",
      "run",
      "build",
    ]);
  });

  it("never falls back to a Windows command shim", () => {
    expect(() => npmInvocation(["audit"], {
      environment: {},
      execPath: "C:\\node\\node.exe",
      platform: "win32",
      pathExists: () => false,
    })).toThrow("Unable to locate npm-cli.js on Windows");
  });

  it("keeps a PATH fallback for direct POSIX use and rejects NUL", () => {
    expect(npmInvocation(["run", "build"], {
      environment: {},
      execPath: "/usr/bin/node",
      platform: "linux",
      pathExists: () => false,
    })).toEqual({ command: "npm", args: ["run", "build"] });
    expect(() => npmInvocation(["bad\0argument"])).toThrow("without NUL");
  });
});
