import { spawnSync } from "node:child_process";
import { mkdirSync } from "node:fs";

export function extractLinuxRpm(archive, destination, {
  environment = process.env,
  run = spawnSync,
} = {}) {
  mkdirSync(destination, { recursive: true });
  // rpm-rs omits the legacy archive-size tag. rpm2cpio 4.18 compares its
  // output size against that absent tag and exits 1 even for a valid payload.
  // rpm2archive reads the actual RPM file entries without that assumption.
  const payload = run("rpm2archive", ["-n", archive], {
    env: environment,
    encoding: null,
    maxBuffer: 256 * 1024 * 1024,
  });
  assertResult("rpm2archive", payload);
  const unpack = run("tar", ["--extract", "--file", "-", "--no-same-owner"], {
    cwd: destination,
    env: environment,
    input: payload.stdout,
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
  });
  assertResult("tar", unpack);
}

function assertResult(command, result) {
  if (result.error) throw result.error;
  if (result.signal || result.status !== 0) {
    throw new Error(`${command} failed: ${result.signal ?? `exit code ${result.status ?? 1}`}${result.stderr?.length ? `\n${result.stderr}` : ""}`);
  }
}
