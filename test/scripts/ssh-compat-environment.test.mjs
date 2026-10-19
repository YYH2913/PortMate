import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const projectRoot = resolve(import.meta.dirname, "../..");

describe("SSH compatibility environment", () => {
  it("pins safe-commit support and exercises fail-closed peers instead of skipping them", () => {
    const matrix = JSON.parse(readFileSync(
      resolve(projectRoot, "test/compat/ssh-server-matrix.json"), "utf8",
    ));
    expect(matrix).toHaveLength(21);
    expect(matrix.every(entry => typeof entry.sftpNoReplace === "boolean")).toBe(true);
    expect(matrix.filter(entry => !entry.sftpNoReplace).map(entry => entry.name)).toEqual([
      "paramiko-5.0.0-debian-bookworm",
      "sftpgo-2.6.6",
      "sftpgo-2.7.5",
      "rclone-1.74.4",
      "erlang-otp-25.2.3-debian-bookworm",
      "erlang-otp-27.3.4.1-debian-trixie",
    ]);
    for (const entry of matrix) {
      expect(entry.sftpNoReplaceError === undefined).toBe(entry.sftpNoReplace);
      if (!entry.sftpNoReplace) expect(entry.sftpNoReplaceError.length).toBeGreaterThan(0);
    }
    expect(matrix.find(entry => entry.name === "rclone-1.74.4")?.sftpNoReplaceError)
      .toBe("Operation unsupported");
    const runner = readFileSync(resolve(projectRoot, "test/tooling/ssh-server-compat-check.mjs"), "utf8");
    expect(runner).toContain('PORTMATE_COMPAT_SFTP_NO_REPLACE: entry.sftpNoReplace ? "1" : "0"');
    expect(runner).toContain("verifiedSafeCommitRejections:");
  });

  it("provides Python for safe remote directory cleanup on shell-capable servers", () => {
    const matrix = JSON.parse(readFileSync(
      resolve(projectRoot, "test/compat/ssh-server-matrix.json"), "utf8",
    ));
    const shellServers = matrix.filter(entry => (entry.protocols ?? ["sftp", "scp"]).includes("scp"));
    expect(shellServers.length).toBeGreaterThan(0);
    for (const { name, dockerfile } of shellServers) {
      const source = readFileSync(resolve(projectRoot, dockerfile), "utf8");
      const install = source.match(/(?:apk add|apt-get install|dnf install)[\s\S]*?(?=\s*&&)/)?.[0];
      expect(install, `${name}: missing Python required by safe remote deletion`)
        .toMatch(/\bpython3\b/);
    }
  });
});
