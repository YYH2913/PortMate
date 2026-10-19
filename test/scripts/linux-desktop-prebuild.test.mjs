import { describe, expect, it, vi } from "vitest";
import { prebuildLinuxDesktop } from "../tooling/linux-desktop-prebuild.mjs";

describe("native Linux desktop prebuild", () => {
  it("builds the real sidecar and desktop in the clean launch environment", () => {
    const run = vi.fn(() => ({ status: 0 }));
    prebuildLinuxDesktop({ platform: "linux", environment: {
      PATH: "/usr/bin", CARGO_TARGET_DIR: "/build/cache", PKG_CONFIG_PATH: "/build/pkgconfig",
      LD_LIBRARY_PATH: "/python/lib", LD_PRELOAD: "/ide/injected.so",
    }, run });
    expect(["npm", process.execPath]).toContain(run.mock.calls[0][0]);
    expect(run.mock.calls[0][1].slice(-2)).toEqual(["run", "sidecar:dev"]);
    expect(run.mock.calls[1].slice(0, 2)).toEqual(["cargo", ["build", "--locked", "-p", "portmate", "--bin", "portmate"]]);
    for (const [, , options] of run.mock.calls) {
      expect(options.env).toMatchObject({ CARGO_TARGET_DIR: "/build/cache", PKG_CONFIG_PATH: "/build/pkgconfig" });
      expect(options.env).not.toHaveProperty("LD_LIBRARY_PATH");
      expect(options.env).not.toHaveProperty("LD_PRELOAD");
      expect(options.timeout).toBe(900_000);
    }
  });

  it.each([{ status: 1 }, { status: null, signal: "SIGTERM" }, { status: null, error: new Error("build timeout") }])("does not mask a failed prerequisite: %j", result => {
    const run = vi.fn(() => result);
    expect(() => prebuildLinuxDesktop({ platform: "linux", environment: {}, run })).toThrow("Linux desktop prebuild failed");
    expect(run).toHaveBeenCalledTimes(1);
  });

  it("refuses to run on another platform", () => {
    const run = vi.fn();
    expect(() => prebuildLinuxDesktop({ platform: "win32", run })).toThrow("requires Linux");
    expect(run).not.toHaveBeenCalled();
  });
});
