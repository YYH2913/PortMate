import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../", import.meta.url));
const result = spawnSync("cargo", [
  "test", "--locked", "--manifest-path", "test/tooling/iota-crypto-check/Cargo.toml",
  "--target-dir", "target/stronghold-crypto-check", "--", "--test-threads=4",
], { cwd: root, stdio: "inherit", env: process.env, windowsHide: true });
if (result.error) throw result.error;
if (result.signal) throw new Error(`Stronghold crypto regression terminated by ${result.signal}`);
process.exitCode = result.status ?? 1;
