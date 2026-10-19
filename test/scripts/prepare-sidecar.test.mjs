import { spawnSync } from "node:child_process";
import {
  copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { afterEach, describe, expect, it } from "vitest";

const projectRoot = resolve(import.meta.dirname, "../..");
const sourcePath = join(projectRoot, "scripts", "prepare-sidecar.mjs");
const temporaryRoots = [];
const host = command("rustc", ["-vV"]).stdout.match(/^host: (.+)$/m)?.[1].trim();
const crossTarget = "x86_64-pc-windows-gnu";
const crossEnv = windowsCrossEnvironment();

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe("sidecar target and artifact consistency", () => {
  const staticHost = "x86_64-unknown-linux-gnu";
  it.each([
    ["implicit host", {}, staticHost, false, false],
    ["Cargo host", { CARGO_BUILD_TARGET: staticHost }, staticHost, true, false],
    ["Tauri host", { TAURI_ENV_TARGET_TRIPLE: staticHost }, staticHost, true, false],
    ["Cargo cross", { CARGO_BUILD_TARGET: crossTarget }, crossTarget, true, false],
    ["Tauri cross", { TAURI_ENV_TARGET_TRIPLE: crossTarget }, crossTarget, true, false],
    ["Tauri host overrides Cargo cross", {
      TAURI_ENV_TARGET_TRIPLE: staticHost, CARGO_BUILD_TARGET: crossTarget,
    }, staticHost, true, false],
    ["Tauri cross overrides Cargo host", {
      TAURI_ENV_TARGET_TRIPLE: crossTarget, CARGO_BUILD_TARGET: staticHost,
    }, crossTarget, true, false],
    ["Tauri cross overrides a different Cargo cross", {
      TAURI_ENV_TARGET_TRIPLE: crossTarget, CARGO_BUILD_TARGET: "aarch64-apple-darwin",
    }, crossTarget, true, true],
    ["empty Tauri falls back to Cargo", {
      TAURI_ENV_TARGET_TRIPLE: "", CARGO_BUILD_TARGET: staticHost,
    }, staticHost, true, true],
    ["empty targets retain implicit host", {
      TAURI_ENV_TARGET_TRIPLE: "", CARGO_BUILD_TARGET: "",
    }, staticHost, false, true],
  ])("uses matching Cargo arguments and artifact layout for %s", (_name, configured, target, explicit, release) => {
    const f = fixture({ relativeTargetDir: !release });
    const profile = release ? "release" : "debug";
    const extension = target.includes("windows") ? ".exe" : "";
    const source = join(f.targetDir, ...(explicit ? [target] : []), profile, `portmate-mcp${extension}`);
    const record = join(f.root, "cargo-arguments.json");
    const preload = join(f.root, "record-cargo.mjs");
    // Only the process boundary is substituted. The preparation script is copied
    // byte-for-byte and must choose the expected source, destination and mode.
    writeFileSync(preload, `
      import childProcess from "node:child_process";
      import { mkdirSync, writeFileSync } from "node:fs";
      import { dirname } from "node:path";
      import { syncBuiltinESMExports } from "node:module";
      childProcess.spawnSync = (command, args, options) => {
        if (command === "rustc") return { status: 0, stdout: ${JSON.stringify(`host: ${staticHost}\n`)}, stderr: "" };
        if (command !== "cargo") throw new Error("unexpected process: " + command);
        writeFileSync(${JSON.stringify(record)}, JSON.stringify({ args, cwd: options.cwd }));
        mkdirSync(dirname(${JSON.stringify(source)}), { recursive: true });
        writeFileSync(${JSON.stringify(source)}, "current-sidecar");
        return { status: 0 };
      };
      syncBuiltinESMExports();
    `);
    const result = command(process.execPath, ["--import", pathToFileURL(preload).href, f.script, ...(release ? ["--release"] : [])], {
      cwd: f.root, env: { ...f.env, ...configured },
    });
    successful(result);
    expect(JSON.parse(readFileSync(record, "utf8"))).toEqual({
      args: ["build", "--locked", "-p", "portmate-mcp", ...(explicit ? ["--target", target] : []), ...(release ? ["--release"] : [])],
      cwd: f.root,
    });
    expect(readFileSync(preparedPath(f, target), "utf8")).toBe("current-sidecar");
  });

  it.each([
    ["implicit host", {}, false],
    ["explicit Cargo host", () => ({ CARGO_BUILD_TARGET: host }), false],
    ["explicit Tauri host", () => ({ TAURI_ENV_TARGET_TRIPLE: host }), false],
    ["Tauri host overriding an invalid Cargo target", () => ({
      TAURI_ENV_TARGET_TRIPLE: host, CARGO_BUILD_TARGET: "portmate-invalid-target",
    }), true],
  ])("prepares a fresh native binary with real Cargo: %s", (_name, configured, release) => {
    expect(host, "rustc must report its native target").toBeTruthy();
    const f = fixture({ relativeTargetDir: !release, lock: true });
    const env = typeof configured === "function" ? configured() : configured;
    const explicit = Boolean(env.CARGO_BUILD_TARGET || env.TAURI_ENV_TARGET_TRIPLE);
    const result = prepare(f, env, release);
    successful(result);
    const built = join(f.targetDir, ...(explicit ? [host] : []), release ? "release" : "debug", binaryName(host));
    expect(readFileSync(preparedPath(f, host))).toEqual(readFileSync(built));
    expect(command(preparedPath(f, host), [], { cwd: f.root, env: f.env }).stdout.trim()).toBe("old-sidecar");
  }, 60_000);

  it.each(["CARGO_BUILD_TARGET", "TAURI_ENV_TARGET_TRIPLE"])("never copies a stale host binary with %s", (variable) => {
    const f = fixture({ relativeTargetDir: true, lock: true });
    // Leave both layouts cached. TAURI_ENV_TARGET_TRIPLE used to build the
    // untargeted layout while copying this older targeted executable.
    successful(prepare(f));
    successful(prepare(f, { CARGO_BUILD_TARGET: host }));
    const stale = readFileSync(preparedPath(f, host));
    writeFileSync(join(f.root, "src", "main.rs"), 'fn main() { println!("new-sidecar-current-source"); }\n');
    const result = prepare(f, { [variable]: host });
    successful(result);
    const prepared = readFileSync(preparedPath(f, host));
    expect(prepared.equals(stale)).toBe(false);
    expect(prepared).toEqual(readFileSync(join(f.targetDir, host, "debug", binaryName(host))));
    expect(command(preparedPath(f, host), [], { cwd: f.root, env: f.env }).stdout.trim()).toBe("new-sidecar-current-source");
    expect(command(join(f.targetDir, "debug", binaryName(host)), [], { cwd: f.root, env: f.env }).stdout.trim()).toBe("old-sidecar");
  }, 60_000);

  it.skipIf(!crossEnv).each(["CARGO_BUILD_TARGET", "TAURI_ENV_TARGET_TRIPLE"])("prepares a fresh Windows binary with real Cargo: %s", (variable) => {
    const f = fixture({ lock: true });
    successful(prepare(f, { ...crossEnv, [variable]: crossTarget }));
    const prepared = readFileSync(preparedPath(f, crossTarget));
    expect(prepared.subarray(0, 2).toString()).toBe("MZ");
    expect(prepared).toEqual(readFileSync(join(f.targetDir, crossTarget, "debug", binaryName(crossTarget))));
  }, 60_000);

  it.skipIf(!crossEnv)("Tauri's cross target builds the current sidecar despite native Cargo defaults and stale caches", () => {
    const f = fixture({ lock: true });
    successful(prepare(f));
    successful(prepare(f, { ...crossEnv, CARGO_BUILD_TARGET: crossTarget }));
    const stale = readFileSync(preparedPath(f, crossTarget));
    writeFileSync(join(f.root, "src", "main.rs"), 'fn main() { println!("new-cross-sidecar-current-source"); }\n');
    successful(prepare(f, { ...crossEnv, TAURI_ENV_TARGET_TRIPLE: crossTarget, CARGO_BUILD_TARGET: host }));
    const prepared = readFileSync(preparedPath(f, crossTarget));
    expect(prepared.equals(stale)).toBe(false);
    expect(prepared.subarray(0, 2).toString()).toBe("MZ");
    expect(prepared).toEqual(readFileSync(join(f.targetDir, crossTarget, "debug", binaryName(crossTarget))));
    expect(command(join(f.targetDir, "debug", binaryName(host)), [], { cwd: f.root, env: f.env }).stdout.trim()).toBe("old-sidecar");
  }, 60_000);
});

function fixture({ relativeTargetDir = false, lock = false } = {}) {
  // macOS resolves /var to /private/var in import.meta.url. Use one canonical
  // root, and exercise URL escaping as well as Windows drive-letter imports.
  const root = realpathSync(mkdtempSync(join(tmpdir(), "portmate sidecar # target ")));
  temporaryRoots.push(root);
  mkdirSync(join(root, "scripts"));
  mkdirSync(join(root, "src"));
  const script = join(root, "scripts", "prepare-sidecar.mjs");
  copyFileSync(sourcePath, script);
  expect(readFileSync(script)).toEqual(readFileSync(sourcePath));
  writeFileSync(join(root, "Cargo.toml"), '[package]\nname = "portmate-mcp"\nversion = "0.0.0"\nedition = "2021"\n\n[workspace]\n');
  writeFileSync(join(root, "src", "main.rs"), 'fn main() { println!("old-sidecar"); }\n');
  const targetDir = join(root, "cargo output");
  const env = {
    ...process.env, CARGO_TARGET_DIR: relativeTargetDir ? "cargo output" : targetDir,
    CARGO_NET_OFFLINE: "true", CARGO_INCREMENTAL: "0",
  };
  for (const key of [
    "CARGO_BUILD_TARGET", "TAURI_ENV_TARGET_TRIPLE", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER",
    "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_RUSTFLAGS",
  ]) delete env[key];
  if (lock) successful(command("cargo", ["generate-lockfile", "--offline"], { cwd: root, env }));
  return { root, script, targetDir, env };
}

function prepare(f, configured = {}, release = false) {
  return command(process.execPath, [f.script, ...(release ? ["--release"] : [])], {
    cwd: f.root, env: { ...f.env, ...configured },
  });
}

function preparedPath(f, target) {
  return join(f.root, "src-tauri", "binaries", `portmate-mcp-${target}${target.includes("windows") ? ".exe" : ""}`);
}

function binaryName(target) {
  return `portmate-mcp${target.includes("windows") ? ".exe" : ""}`;
}

function command(executable, args, options = {}) {
  return spawnSync(executable, args, {
    encoding: "utf8", timeout: 45_000, maxBuffer: 2 * 1024 * 1024, ...options,
  });
}

function successful(result) {
  expect(result.error, result.stderr).toBeUndefined();
  expect(result.status, `${result.stdout}\n${result.stderr}`).toBe(0);
}

function windowsCrossEnvironment() {
  if (host === crossTarget) return null;
  const targets = command("rustup", ["target", "list", "--installed"]);
  if (targets.status !== 0 || !targets.stdout.split(/\r?\n/u).includes(crossTarget)) return null;
  const bundled = join(projectRoot, "target", "mingw-toolchain", "root", "usr", "bin", "x86_64-w64-mingw32-gcc-posix");
  for (const compiler of [
    process.env.CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER,
    ...(existsSync(bundled) ? [bundled] : []),
    "x86_64-w64-mingw32-gcc",
  ].filter(Boolean)) {
    if (command(compiler, ["--version"]).status !== 0) continue;
    return {
      CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER: compiler,
      PATH: `${dirname(compiler)}${delimiter}${process.env.PATH || ""}`,
    };
  }
  return null;
}
