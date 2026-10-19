import { describe, expect, it } from "vitest";
import { nativeRustTestArguments } from "../tooling/native-rust-tests.mjs";

describe("native Rust CI test profiles", () => {
  it("explicitly includes test-only probes in every suite without enabling them for package builds", () => {
    for (const platform of ["linux", "darwin", "win32"]) {
      for (const suite of ["portable-vault", "release-upgrade", "workspace"]) {
        const args = nativeRustTestArguments(suite, platform);
        const index = args.indexOf("--features");
        expect(index).toBeGreaterThan(0);
        expect(args[index + 1]).toBe("portmate/test-probes");
        expect(index).toBeLessThan(args.indexOf("--"));
      }
    }
  });

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
