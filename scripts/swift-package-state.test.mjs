import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  isRecoverableSwiftPackageFailure,
  resetSwiftPackageState,
  runSwiftBuildWithRecovery,
} from "./swift-package-state.mjs";

const roots = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe("Swift package build state", () => {
  it("removes restored SCM state without deleting compiled products", () => {
    const scratch = temporaryRoot();
    for (const entry of ["checkouts", "repositories"]) mkdirSync(join(scratch, entry));
    writeFileSync(join(scratch, "workspace-state.json"), "{}");
    writeFileSync(join(scratch, "release.yaml"), "compiled");

    resetSwiftPackageState(scratch);

    expect(existsSync(join(scratch, "checkouts"))).toBe(false);
    expect(existsSync(join(scratch, "repositories"))).toBe(false);
    expect(existsSync(join(scratch, "workspace-state.json"))).toBe(false);
    expect(existsSync(join(scratch, "release.yaml"))).toBe(true);
  });

  it("cleans scratch and shared SCM caches before one recovery retry", async () => {
    const root = temporaryRoot();
    const scratch = join(root, "scratch");
    const cache = join(root, "cache");
    mkdirSync(join(scratch, "repositories"), { recursive: true });
    mkdirSync(join(cache, "repositories"), { recursive: true });
    mkdirSync(join(cache, "manifests"), { recursive: true });
    const build = vi.fn()
      .mockRejectedValueOnce(new Error(
        "Failed to clone repository: fatal: repository '/tmp/swift-atomics' does not exist",
      ))
      .mockResolvedValueOnce("built");
    const onRetry = vi.fn();

    await expect(runSwiftBuildWithRecovery({ scratch, cache, build, onRetry }))
      .resolves.toBe("built");
    expect(build).toHaveBeenCalledTimes(2);
    expect(onRetry).toHaveBeenCalledOnce();
    expect(existsSync(join(scratch, "repositories"))).toBe(false);
    expect(existsSync(join(cache, "repositories"))).toBe(false);
    expect(existsSync(join(cache, "manifests"))).toBe(false);
  });

  it("does not retry compiler failures", async () => {
    const root = temporaryRoot();
    const build = vi.fn().mockRejectedValue(new Error("Swift compiler error in main.swift"));
    await expect(runSwiftBuildWithRecovery({
      scratch: join(root, "scratch"),
      cache: join(root, "cache"),
      build,
    })).rejects.toThrow("compiler error");
    expect(build).toHaveBeenCalledOnce();
  });

  it("recognizes the missing repository diagnostics emitted by SwiftPM", () => {
    expect(isRecoverableSwiftPackageFailure(
      new Error("fatal: repository '/build/repositories/swift-system-5815d4b7' does not exist"),
    )).toBe(true);
    expect(isRecoverableSwiftPackageFailure(new Error("type mismatch"))).toBe(false);
  });
});

function temporaryRoot() {
  const root = mkdtempSync(join(tmpdir(), "portmate-swift-state-"));
  roots.push(root);
  return root;
}
