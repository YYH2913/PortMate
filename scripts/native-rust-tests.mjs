import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const suiteArguments = new Map([
  ["portable-vault", [
    "test",
    "--locked",
    "-p",
    "portmate",
    "--no-default-features",
    "tests::portable_vault_tests::portable_vault_cross_process_fault_matrix",
    "--",
    "--exact",
    "--nocapture",
    "--test-threads=1",
  ]],
  ["release-upgrade", [
    "test",
    "--locked",
    "-p",
    "portmate",
    "--no-default-features",
    "tests::release_upgrade_tests::",
    "--",
    "--test-threads=1",
  ]],
  ["workspace", [
    "test",
    "--locked",
    "--workspace",
    "--no-default-features",
    "--",
    "--test-threads=1",
    "--skip",
    "tests::portable_vault_tests::portable_vault_cross_process_fault_matrix",
  ]],
]);

export function nativeRustTestArguments(suite, platform = process.platform) {
  const configured = suiteArguments.get(suite);
  if (!configured) {
    throw new Error(`unknown native Rust test suite: ${String(suite)}`);
  }
  if (typeof platform !== "string" || !platform) {
    throw new Error("native Rust test platform must be a non-empty string");
  }
  const args = [...configured];
  if (platform === "win32") {
    // Vendored OpenSSL and libsodium must use the same release CRT as Rust on MSVC.
    args.splice(args.indexOf("--no-default-features") + 1, 0, "--release");
  }
  return args;
}

function main() {
  const result = spawnSync("cargo", nativeRustTestArguments(process.argv[2]), {
    cwd: projectRoot,
    stdio: "inherit",
    windowsHide: true,
  });
  if (result.error) {
    console.error(`failed to execute cargo: ${result.error.message}`);
    process.exitCode = 1;
    return;
  }
  if (result.signal) {
    console.error(`cargo was terminated by ${result.signal}`);
    process.exitCode = 1;
    return;
  }
  process.exitCode = result.status ?? 1;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
