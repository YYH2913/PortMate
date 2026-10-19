import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { cargoLockPinsPackage } from "../tooling/cargo-lock-state.mjs";
import { ALLOWED_RUSTSEC_VULNERABILITIES } from "../tooling/rust-dependency-audit.mjs";

const projectRoot = resolve(import.meta.dirname, "../..");
const source = path => readFileSync(resolve(projectRoot, path), "utf8");

describe("libsodium build dependency patch", () => {
  it("uses the provenance-pinned local bindings with patched ZIP and no new exceptions", () => {
    const workspace = source("Cargo.toml");
    expect(workspace).toContain('exclude = ["crates/libsodium-sys-stable"]');
    expect(workspace).toMatch(/\[patch\.crates-io\][\s\S]*libsodium-sys-stable = \{ path = "crates\/libsodium-sys-stable" \}/);
    const manifest = source("crates/libsodium-sys-stable/Cargo.toml");
    expect(manifest).toContain('version = "1.24.0"');
    expect(manifest).toContain('zip = { version = "9.0.5", default-features = false, features = ["deflate"] }');
    const lock = source("Cargo.lock");
    expect(cargoLockPinsPackage(lock, "zip", "9.0.5")).toBe(true);
    expect(lock).not.toMatch(/name = "zip"\r?\nversion = "8\./);
    expect(ALLOWED_RUSTSEC_VULNERABILITIES).toEqual([
      "RUSTSEC-2023-0071:rsa@0.10.0-rc.18",
    ]);
  });

  it("retains the signed upstream native source archive without modifications", () => {
    const archive = readFileSync(resolve(projectRoot, "crates/libsodium-sys-stable/LATEST.tar.gz"));
    expect(createHash("sha256").update(archive).digest("hex"))
      .toBe("b20a92e7ec25b285eafa349d721a5bb27e3a8ba94c0816630a127883f1d1b3ab");
    expect(source("crates/libsodium-sys-stable/LATEST.tar.gz.minisig"))
      .toContain("untrusted comment:");
    const build = source("crates/libsodium-sys-stable/build.rs");
    expect(build).toContain("retrieve_and_verify_archive");
    expect(build).toContain("minisign_verify");
  });
});
