import { execFileSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

export const PORTABLE_CROSS_CRATES = Object.freeze([
  "portmate-core",
  "portmate-kdf",
  "russh-sftp",
  "portmate-keyring",
  "portmate-process-watchdog",
]);

export const PORTABLE_CROSS_TARGETS = Object.freeze([
  Object.freeze({ label: "Windows GNU", triple: "x86_64-pc-windows-gnu" }),
  Object.freeze({ label: "macOS Apple Silicon", triple: "aarch64-apple-darwin" }),
  Object.freeze({ label: "FreeBSD x86_64", triple: "x86_64-unknown-freebsd" }),
]);

export function parseInstalledRustTargets(output) {
  if (typeof output !== "string") {
    throw new TypeError("rustup target output must be a string");
  }
  return new Set(output.split(/\r?\n/).map((line) => line.trim()).filter(Boolean));
}

export function buildPortableCrossCheckPlan() {
  const packageArgs = PORTABLE_CROSS_CRATES.flatMap((crate) => ["-p", crate]);
  return PORTABLE_CROSS_TARGETS.map(({ label, triple }) => ({
    label,
    target: triple,
    command: "cargo",
    args: ["check", "--locked", "--all-targets", ...packageArgs, "--target", triple],
  }));
}

export function missingPortableCrossTargets(installedTargets) {
  if (!(installedTargets instanceof Set)) {
    throw new TypeError("installed Rust targets must be a Set");
  }
  return PORTABLE_CROSS_TARGETS
    .map(({ triple }) => triple)
    .filter((target) => !installedTargets.has(target));
}

function main() {
  const installedTargets = parseInstalledRustTargets(execFileSync(
    "rustup",
    ["target", "list", "--installed"],
    { cwd: projectRoot, encoding: "utf8" },
  ));
  const missingTargets = missingPortableCrossTargets(installedTargets);
  if (missingTargets.length > 0) {
    throw new Error(
      `portable cross-check requires missing Rust targets: ${missingTargets.join(", ")}\n`
      + `Install them with: rustup target add ${missingTargets.join(" ")}`,
    );
  }

  for (const entry of buildPortableCrossCheckPlan()) {
    console.log(`Checking portable crates for ${entry.label} (${entry.target})`);
    execFileSync(entry.command, entry.args, {
      cwd: projectRoot,
      stdio: "inherit",
    });
  }
  console.log(
    `Portable Rust cross-check passed (${PORTABLE_CROSS_CRATES.length} crates, ${PORTABLE_CROSS_TARGETS.length} targets)`,
  );
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
