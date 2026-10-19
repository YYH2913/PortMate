import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { extractLinuxRpm } from "../tooling/linux-rpm-extract.mjs";

const roots = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function destination() {
  const root = mkdtempSync(join(tmpdir(), "portmate rpm extract "));
  roots.push(root);
  return join(root, "rpm payload");
}

describe("Linux RPM extraction", () => {
  it("uses RPM file entries and passes their binary tar payload unchanged to the extractor", () => {
    const archive = "/build with spaces/PortMate.rpm";
    const root = destination();
    const environment = { PATH: "/usr/bin", TMPDIR: "/isolated temporary files" };
    const stdout = Buffer.from([0, 255, 1, 128]);
    const run = vi.fn().mockReturnValueOnce({ status: 0, stdout }).mockReturnValueOnce({ status: 0 });

    extractLinuxRpm(archive, root, { environment, run });

    expect(run.mock.calls).toEqual([
      ["rpm2archive", ["-n", archive], {
        env: environment, encoding: null, maxBuffer: 256 * 1024 * 1024,
      }],
      ["tar", ["--extract", "--file", "-", "--no-same-owner"], {
        cwd: root, env: environment, input: stdout, encoding: "utf8", maxBuffer: 16 * 1024 * 1024,
      }],
    ]);
  });

  it.each([
    { status: 1, stderr: Buffer.from("invalid RPM header") },
    { status: null, signal: "SIGTERM" },
    { status: 0, signal: "SIGTERM" },
    { status: null, error: new Error("rpm2archive missing") },
  ])("does not extract a failed converter's output: %j", result => {
    const run = vi.fn(() => result);
    expect(() => extractLinuxRpm("/archive.rpm", destination(), { run })).toThrow();
    expect(run).toHaveBeenCalledTimes(1);
  });

  it.each([
    { status: 2, stderr: "truncated tar archive" },
    { status: null, signal: "SIGTERM" },
    { status: null, error: new Error("tar spawn failed") },
  ])("does not mask extraction failure: %j", result => {
    const run = vi.fn().mockReturnValueOnce({ status: 0, stdout: Buffer.alloc(0) }).mockReturnValueOnce(result);
    expect(() => extractLinuxRpm("/archive.rpm", destination(), { run })).toThrow();
    expect(run).toHaveBeenCalledTimes(2);
  });
});
