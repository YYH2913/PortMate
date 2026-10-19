import { spawnSync } from "node:child_process";
import { describe, expect, it, vi } from "vitest";
import { mountVerifiedMacDmg } from "../tooling/macos-dmg-mount.mjs";

const options = {
  dmg: "/build with spaces/PortMate.dmg",
  mountPoint: "/audit with spaces/mounted-dmg",
  cwd: "/project",
  environment: { PATH: "/usr/bin", TMPDIR: "/isolated temporary files" },
};

describe("verified macOS DMG mount", () => {
  it("supplies one license acceptance without changing the read-only mount or environment", () => {
    const run = vi.fn(() => ({ status: 0 }));
    mountVerifiedMacDmg({ ...options, run });
    expect(run).toHaveBeenCalledExactlyOnceWith("hdiutil", [
      "attach", options.dmg, "-readonly", "-nobrowse", "-mountpoint", options.mountPoint,
    ], {
      cwd: options.cwd,
      env: options.environment,
      input: "Y\n",
      stdio: ["pipe", "inherit", "inherit"],
      timeout: 120_000,
    });
  });

  it("delivers the bounded answer and EOF to an actual child process", () => {
    mountVerifiedMacDmg({
      ...options,
      run: (_command, _args, spawnOptions) => spawnSync(process.execPath, [
        "--input-type=module", "-e",
        'import { readFileSync } from "node:fs"; if (!readFileSync(0).equals(Buffer.from("Y\\n"))) process.exit(2);',
      ], { ...spawnOptions, cwd: process.cwd(), env: process.env }),
    });
  });

  it.each([
    { status: 1 },
    { status: null, signal: "SIGTERM" },
    { status: 0, signal: "SIGTERM" },
  ])("does not mask mount failure: %j", result => {
    expect(() => mountVerifiedMacDmg({ ...options, run: () => result })).toThrow("hdiutil attach failed");
  });

  it("preserves process errors, including timeouts", () => {
    const error = Object.assign(new Error("mount timeout"), { code: "ETIMEDOUT" });
    expect(() => mountVerifiedMacDmg({ ...options, run: () => ({ status: null, error }) })).toThrow(error);
  });
});
