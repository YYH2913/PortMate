import { spawnSync } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, rmSync, statSync, writeFileSync } from "node:fs";
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
  it.skipIf(process.platform !== "linux")("preserves archive modes even with a restrictive host umask", () => {
    const root = destination();
    const source = join(root, "source");
    mkdirSync(source, { recursive: true });
    for (const [name, mode] of [["tool", 0o755], ["LICENSE", 0o644]]) {
      writeFileSync(join(source, name), name);
      chmodSync(join(source, name), mode);
    }
    const archive = join(root, "payload.tar");
    const pack = spawnSync("tar", ["--create", "--file", archive, "--directory", source, "tool", "LICENSE"], {
      encoding: "utf8", timeout: 10_000,
    });
    expect(pack.status, pack.stderr).toBe(0);
    const extracted = join(root, "extracted");
    // Set umask in a separate process, never in Vitest's shared worker.
    const child = spawnSync(process.execPath, ["--input-type=module", "-e", `
      import { spawnSync } from "node:child_process";
      import { readFileSync } from "node:fs";
      import { extractLinuxRpm } from ${JSON.stringify(new URL("../tooling/linux-rpm-extract.mjs", import.meta.url).href)};
      process.umask(0o077);
      extractLinuxRpm("fixture.rpm", process.argv[2], {
        run: (command, args, options) => command === "rpm2archive"
          ? { status: 0, stdout: readFileSync(process.argv[1]) }
          : spawnSync(command, args, options),
      });
    `, archive, extracted], { encoding: "utf8", timeout: 10_000 });
    expect(child.status, child.stderr).toBe(0);
    expect(statSync(join(extracted, "tool")).mode & 0o777).toBe(0o755);
    expect(statSync(join(extracted, "LICENSE")).mode & 0o777).toBe(0o644);
  });

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
      ["tar", ["--extract", "--file", "-", "--no-same-owner", "--same-permissions"], {
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
