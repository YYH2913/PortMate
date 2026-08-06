import { describe, expect, it } from "vitest";
import {
  PORTABLE_CROSS_CRATES,
  PORTABLE_CROSS_TARGETS,
  buildPortableCrossCheckPlan,
  missingPortableCrossTargets,
  parseInstalledRustTargets,
} from "./portable-cross-check.mjs";

const expectedCrates = [
  "portmate-core",
  "portmate-kdf",
  "russh-sftp",
  "portmate-keyring",
  "portmate-process-watchdog",
];
const expectedTargets = [
  "x86_64-pc-windows-gnu",
  "aarch64-apple-darwin",
  "x86_64-unknown-freebsd",
];

describe("portable Rust cross-check", () => {
  it("pins the intended crate and target matrix", () => {
    expect(PORTABLE_CROSS_CRATES).toEqual(expectedCrates);
    expect(PORTABLE_CROSS_TARGETS.map(({ triple }) => triple)).toEqual(expectedTargets);
  });

  it("builds locked cargo checks for every target", () => {
    const plan = buildPortableCrossCheckPlan();
    expect(plan).toHaveLength(expectedTargets.length);
    for (const [index, entry] of plan.entries()) {
      expect(entry.command).toBe("cargo");
      expect(entry.target).toBe(expectedTargets[index]);
      expect(entry.args).toEqual([
        "check",
        "--locked",
        ...expectedCrates.flatMap((crate) => ["-p", crate]),
        "--target",
        expectedTargets[index],
      ]);
    }
  });

  it("parses rustup output and reports only missing targets", () => {
    const installed = parseInstalledRustTargets(
      `x86_64-pc-windows-gnu\r\naarch64-apple-darwin\n\n`,
    );
    expect(missingPortableCrossTargets(installed)).toEqual(["x86_64-unknown-freebsd"]);
    expect(missingPortableCrossTargets(new Set(expectedTargets))).toEqual([]);
  });

  it("rejects malformed helper input", () => {
    expect(() => parseInstalledRustTargets(null)).toThrow("must be a string");
    expect(() => missingPortableCrossTargets(expectedTargets)).toThrow("must be a Set");
  });
});
