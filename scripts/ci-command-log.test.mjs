import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { afterEach, describe, expect, it } from "vitest";
import { parseLoggedCommandArguments } from "./ci-command-log.mjs";

const helper = resolve(import.meta.dirname, "ci-command-log.mjs");
const temporaryRoots = [];

function workflowStep(workflow, name) {
  const marker = `      - name: ${name}`;
  const start = workflow.indexOf(marker);
  if (start < 0) throw new Error(`missing workflow step: ${name}`);
  const next = workflow.indexOf("\n      - ", start + marker.length);
  return workflow.slice(start, next < 0 ? undefined : next);
}

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) {
    rmSync(root, { recursive: true, force: true });
  }
});

describe("CI command logging", () => {
  it("keeps a structured argv and rejects incomplete commands", () => {
    expect(parseLoggedCommandArguments(["diagnostics/build.log", "npm", "run", "build"]))
      .toMatchObject({ command: "npm", args: ["run", "build"] });
    expect(parseLoggedCommandArguments(["diagnostics/build.log", "npm"]).command).toBe("npm");
    expect(() => parseLoggedCommandArguments(["diagnostics/build.log"])).toThrow("Usage:");
    expect(() => parseLoggedCommandArguments(["bad\0path", "npm"])).toThrow("without NUL");
  });

  it("tees stdout and stderr into a path containing spaces and preserves failure status", () => {
    const root = mkdtempSync(join(tmpdir(), "portmate ci command log "));
    temporaryRoots.push(root);
    const logPath = join(root, "nested reports", "package audit.log");
    const result = spawnSync(process.execPath, [
      helper,
      logPath,
      process.execPath,
      "-e",
      "process.stdout.write('audit-out\\n' + 'x'.repeat(262144)); process.stderr.write('audit-error\\n'); process.exitCode = 7",
    ], { encoding: "utf8" });

    expect(result.status).toBe(7);
    expect(result.stdout).toContain("audit-out");
    expect(result.stderr).toContain("audit-error");
    const log = readFileSync(logPath, "utf8");
    expect(log).toContain("audit-out");
    expect(log).toContain("audit-error");
    expect(log.match(/x/g)).toHaveLength(262144);
  });

  it("retains both native credential fault gates as CI diagnostics", () => {
    const workflow = readFileSync(
      resolve(import.meta.dirname, "..", ".github", "workflows", "native-ci.yml"),
      "utf8",
    );
    expect(workflow).toContain(
      "target/native-ci/portable-vault.log npm run test:portable-vault",
    );
    expect(workflow).toContain(
      "target/native-ci/native-keyring.log npm run test:native-keyring",
    );
    expect(workflow).toContain(
      "target/native-ci/workspace-tests.log",
    );
    expect(workflow).toContain(
      "target/native-ci/libssh-gssapi-build.log npm run test:libssh-gssapi-build",
    );
    expect(workflow).toContain(
      "target/native-ci/ssh-gssapi-compat.log npm run test:ssh-gssapi-compat",
    );
    expect(workflow).toContain(
      "target/native-ci/mcp-swift-client.log npm run test:mcp-swift-client",
    );
    for (const invocation of [
      "dependency-audit.log npm run test:dependency-audit",
      "native-keyring-dependencies.log npm run test:native-keyring-dependencies",
      "rustfmt.log cargo fmt --all -- --check",
      "frontend-tests.log npm test",
      "frontend-build.log npm run build",
      "clippy.log cargo clippy --locked --workspace --all-targets -- -D warnings",
    ]) {
      expect(workflow).toContain(`target/native-ci/${invocation}`);
    }
    for (const client of [
      "typescript",
      "python",
      "go",
      "rust",
      "ruby",
      "java",
      "kotlin",
      "csharp",
    ]) {
      expect(workflow).toContain(
        `target/native-ci/mcp-${client}-client.log npm run test:mcp-${client}-client`,
      );
    }
    expect(workflow).toContain("--test-threads=1");
    expect(workflow.match(/swift-actions\/setup-swift@v3/g)).toHaveLength(1);
    expect(workflow.match(/dtolnay\/rust-toolchain@1\.97\.1/g)).toHaveLength(4);
    expect(workflow).not.toContain("dtolnay/rust-toolchain@stable");
    expect(workflow.match(/actions\/checkout@v7/g)).toHaveLength(4);
    expect(workflow.match(/actions\/setup-node@v7/g)).toHaveLength(4);
    expect(workflow.match(/actions\/setup-python@v7/g)).toHaveLength(2);
    expect(workflow.match(/actions\/setup-go@v7/g)).toHaveLength(2);
    expect(workflow.match(/actions\/setup-java@v5/g)).toHaveLength(2);
    expect(workflow.match(/actions\/setup-dotnet@v6/g)).toHaveLength(2);
    expect(workflow.match(/actions\/upload-artifact@v7/g)).toHaveLength(3);
    expect(workflow.match(/(?:^|\n)\s+cache: false/g)).toHaveLength(2);
    expect(workflow).not.toMatch(/actions\/(?:checkout|setup-node)@v4/);
    expect(workflow).not.toMatch(/actions\/(?:setup-python|setup-go)@v5/);
    expect(workflow).not.toMatch(/actions\/setup-java@v4/);
    expect(workflow).not.toMatch(/actions\/setup-dotnet@v4/);
    expect(workflow).not.toMatch(/actions\/upload-artifact@v4/);
    expect(workflow).toContain("if: runner.os == 'Linux'");
    expect(workflow).toContain("- macos-15");
    expect(workflow).not.toContain("swift-actions/setup-swift@v2");
    const swiftSetup = workflowStep(workflow, "Install pinned Swift SDK client toolchain");
    expect(swiftSetup).toContain("if: runner.os != 'Windows'");
    expect(swiftSetup).toContain('swift-version: "6.3.3"');
    expect(workflow).toContain("ilammy/setup-nasm@v1");
    expect(workflow).toContain("if: runner.os == 'Windows'");
    const nativePerl = workflowStep(workflow, "Install native Perl on Windows");
    expect(nativePerl).toContain("shogo82148/actions-setup-perl@v1");
    expect(nativePerl).toContain("distribution: strawberry");
    const opensslSetup = workflowStep(workflow, "Pin native Perl for vendored OpenSSL on Windows");
    expect(opensslSetup).toContain("shell: pwsh");
    expect(opensslSetup).toContain("OPENSSL_SRC_PERL=$perl");
    expect(opensslSetup).toContain("$perlOs -ne 'MSWin32'");
    expect(opensslSetup).toContain("Microsoft Visual Studio\\Installer\\vswhere.exe");
    expect(opensslSetup).toContain("Microsoft.VisualStudio.Component.VC.Tools.x86.x64");
    expect(opensslSetup).toContain("VC\\Tools\\MSVC\\*\\bin\\Hostx64\\x64\\nmake.exe");
    expect(opensslSetup).toContain("$env:GITHUB_PATH");
    expect(opensslSetup).not.toContain("Get-Command nmake.exe");
    expect(opensslSetup).toContain('"MAKE=nmake.exe"');
    expect(opensslSetup).not.toMatch(/(?:^|\n)\s*["']?MAKEFLAGS=/m);
    const sodiumSetup = workflowStep(workflow, "Install release CRT libsodium on Windows");
    expect(sodiumSetup).toContain("Get-Command vcpkg.exe");
    expect(sodiumSetup).toContain("--triplet=x64-windows-static-md");
    expect(sodiumSetup).toContain("x64-windows-static-md\\lib");
    expect(sodiumSetup).toContain("libsodium.lib");
    expect(sodiumSetup).toContain('"SODIUM_LIB_DIR=$sodiumLib"');
    const vcpkgManifest = JSON.parse(readFileSync(
      resolve(import.meta.dirname, "..", "vcpkg.json"),
      "utf8",
    ));
    expect(vcpkgManifest).toEqual({
      $schema: "https://raw.githubusercontent.com/microsoft/vcpkg-tool/main/docs/vcpkg.schema.json",
      name: "portmate-native-ci",
      version: "0.1.1",
      "builtin-baseline": "86dc619bd8d9697405ae5c944b474117ea9457ce",
      dependencies: ["libsodium"],
    });

    const freshnessWorkflow = readFileSync(
      resolve(import.meta.dirname, "..", ".github", "workflows", "mcp-sdk-freshness.yml"),
      "utf8",
    );
    expect(freshnessWorkflow).toContain("actions/checkout@v7");
    expect(freshnessWorkflow).toContain("actions/setup-node@v7");
    expect(freshnessWorkflow).not.toMatch(/actions\/(?:checkout|setup-node)@v4/);
  });

  it("keeps native Rolldown bindings installable on every native runner", () => {
    const lockfile = JSON.parse(readFileSync(
      resolve(import.meta.dirname, "..", "package-lock.json"),
      "utf8",
    ));
    const packages = lockfile.packages;
    expect(packages["node_modules/@rolldown/binding-darwin-arm64"]).toMatchObject({
      os: ["darwin"],
      cpu: ["arm64"],
    });
    expect(packages["node_modules/@rolldown/binding-darwin-arm64"]).not.toHaveProperty("libc");
    expect(packages["node_modules/@rolldown/binding-darwin-x64"]).toMatchObject({
      os: ["darwin"],
      cpu: ["x64"],
    });
    expect(packages["node_modules/@rolldown/binding-darwin-x64"]).not.toHaveProperty("libc");
    expect(packages["node_modules/@rolldown/binding-win32-x64-msvc"]).toMatchObject({
      os: ["win32"],
      cpu: ["x64"],
    });
    expect(packages["node_modules/@rolldown/binding-win32-x64-msvc"]).not.toHaveProperty("libc");
    expect(packages["node_modules/@rolldown/binding-linux-x64-gnu"]).toMatchObject({
      os: ["linux"],
      cpu: ["x64"],
      libc: ["glibc"],
    });
    expect(packages["node_modules/@rolldown/binding-linux-x64-musl"]).toMatchObject({
      os: ["linux"],
      cpu: ["x64"],
      libc: ["musl"],
    });
  });
});
