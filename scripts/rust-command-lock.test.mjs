import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const projectRoot = resolve(import.meta.dirname, "..");

function source(path) {
  return readFileSync(resolve(projectRoot, path), "utf8");
}

describe("Rust command lock boundary", () => {
  it("keeps every MCP SDK bridge build on the workspace lockfile", () => {
    const packageJson = JSON.parse(source("package.json"));
    const scripts = Object.entries(packageJson.scripts)
      .filter(([name, command]) => name.startsWith("test:mcp-") && command.startsWith("cargo build"));

    expect(scripts).toHaveLength(11);
    for (const [name, command] of scripts) {
      expect(command, name).toMatch(/^cargo build --locked -p portmate-mcp /);
    }
  });

  it("locks native CI, sidecar, Tauri, and compatibility workspace commands", () => {
    const expectedPatterns = new Map([
      [".github/workflows/native-ci.yml", [
        /cargo test --locked --workspace/,
        /cargo clippy --locked --workspace/,
      ]],
      ["scripts/prepare-sidecar.mjs", [/\["build", "--locked", "-p", "portmate-mcp"\]/]],
      ["scripts/desktop-dev.mjs", [/"dev",\s*"--",\s*"--locked"/]],
      ["scripts/build-desktop.mjs", [/"build",[\s\S]*"--config",[\s\S]*"--",\s*"--locked"/]],
      ["scripts/libssh-gssapi-build-check.sh", [/cargo test --locked -p libssh-rs/]],
      ["scripts/tmux-version-compat-check.mjs", [/\["build", "--locked", "-p", "portmate"/]],
      ["scripts/tcp-telnet-server-compat-check.mjs", [/"test",\s*"--locked",\s*"-p"/]],
      ["scripts/ssh-gssapi-compat-check.mjs", [/"test",\s*"--locked",\s*"-p"/]],
    ]);

    for (const [path, patterns] of expectedPatterns) {
      const contents = source(path);
      for (const pattern of patterns) expect(contents, path).toMatch(pattern);
    }

    const sshMatrix = source("scripts/ssh-server-compat-check.mjs");
    expect(sshMatrix.match(/"test",\s*"--locked",\s*"-p"/g)).toHaveLength(4);
    expect(sshMatrix.match(/timeout: compatibilityTestTimeoutMs/g)).toHaveLength(4);
    expect(sshMatrix).toContain("const compatibilityTestTimeoutMs = 900_000;");

    const gssapiMatrix = source("scripts/ssh-gssapi-compat-check.mjs");
    const prebuildIndex = gssapiMatrix.indexOf("prebuildRuntimeTest();");
    expect(gssapiMatrix).toContain('"--no-run"');
    expect(prebuildIndex).toBeGreaterThanOrEqual(0);
    expect(prebuildIndex).toBeLessThan(gssapiMatrix.indexOf("for (const entry of selectedMatrix)"));
  });
});
