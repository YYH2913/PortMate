import { describe, expect, it } from "vitest";
import { nativeRustTestArguments } from "./native-rust-tests.mjs";

describe("native Rust CI test profiles", () => {
  it.each(["linux", "darwin"])("keeps the default test profile on %s", (platform) => {
    for (const suite of ["portable-vault", "release-upgrade", "workspace"]) {
      const args = nativeRustTestArguments(suite, platform);
      expect(args.slice(0, 2), suite).toEqual(["test", "--locked"]);
      expect(args, suite).toContain("--no-default-features");
      expect(args, suite).not.toContain("--release");
    }
  });

  it("uses release-linked test binaries for every Windows native suite", () => {
    for (const suite of ["portable-vault", "release-upgrade", "workspace"]) {
      const args = nativeRustTestArguments(suite, "win32");
      expect(args.slice(0, 2), suite).toEqual(["test", "--locked"]);
      expect(args.indexOf("--release"), suite)
        .toBe(args.indexOf("--no-default-features") + 1);
      expect(args.indexOf("--release"), suite).toBeLessThan(args.indexOf("--"));
    }
  });

  it("preserves suite filters and rejects unknown suites", () => {
    expect(nativeRustTestArguments("release-upgrade", "win32"))
      .toContain("tests::release_upgrade_tests::");
    expect(nativeRustTestArguments("portable-vault", "win32"))
      .toContain("--exact");
    expect(nativeRustTestArguments("workspace", "win32"))
      .toContain("--skip");
    expect(() => nativeRustTestArguments("unknown", "linux"))
      .toThrow("unknown native Rust test suite");
    expect(() => nativeRustTestArguments("workspace", ""))
      .toThrow("platform must be a non-empty string");
  });
});
