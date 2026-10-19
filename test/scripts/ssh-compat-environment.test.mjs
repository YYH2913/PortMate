import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const projectRoot = resolve(import.meta.dirname, "../..");

describe("SSH compatibility environment", () => {
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
