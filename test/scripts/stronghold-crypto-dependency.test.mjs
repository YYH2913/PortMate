import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { ALLOWED_RUSTSEC_VULNERABILITIES } from "../tooling/rust-dependency-audit.mjs";

const root = resolve(import.meta.dirname, "../..");
const source = path => readFileSync(resolve(root, path), "utf8");

describe("patched security dependency graph", () => {
  it("removes newly reported vulnerable and yanked versions without adding exceptions", () => {
    const lock = source("Cargo.lock");
    const packages = lock.split("[[package]]").slice(1).map(block => ({
      name: block.match(/^name = "([^"]+)"$/m)?.[1],
      version: block.match(/^version = "([^"]+)"$/m)?.[1],
    }));
    const floors = {
      "crypto-bigint": "0.7.6", ctutils: "0.4.4", "libz-sys": "1.1.31",
      "libsqlite3-sys": "0.39.0", russh: "0.65.0", rustix: "1.1.6",
      tokio: "1.53.3", "async-executor": "1.14.1",
    };
    for (const [name, minimum] of Object.entries(floors)) {
      const versions = packages.filter(pkg => pkg.name === name).map(pkg => pkg.version);
      expect(versions.length, name).toBeGreaterThan(0);
      for (const version of versions) {
        expect(version, name).toMatch(/^\d+\.\d+\.\d+$/);
        const actual = version.split(".").map(Number);
        const floor = minimum.split(".").map(Number);
        const difference = actual.map((value, index) => value - floor[index]).find(value => value !== 0) ?? 0;
        expect(difference, `${name}@${version}`).toBeGreaterThanOrEqual(0);
      }
    }
    expect(ALLOWED_RUSTSEC_VULNERABILITIES).toEqual(["RUSTSEC-2023-0071:rsa@0.10.0-rc.18"]);
  });

  it("keeps the patched fork honest about its origin and unchanged snapshot cryptography", () => {
    const workspace = source("Cargo.toml");
    expect(workspace).toContain('iota-crypto = { path = "crates/iota-crypto" }');
    const manifest = source("crates/iota-crypto/Cargo.toml");
    expect(manifest).toContain('version = "0.23.2"');
    expect(manifest).toMatch(/\[dependencies.k256\]\nversion = "0.14"/);
    const provenance = JSON.parse(source("crates/iota-crypto/PORTMATE_UPSTREAM.json"));
    expect(provenance.crateSha256).toBe("98a38db844c910d78825e173c083f2ef416b69cb091bba8ac1055763c6db065b");
    expect(provenance.unchangedSources["src/ciphers/chacha.rs"]).toBeDefined();
    expect(Object.keys(provenance.unchangedSources).length).toBeGreaterThan(50);
    for (const [path, hash] of Object.entries(provenance.unchangedSources)) {
      expect(createHash("sha256").update(readFileSync(resolve(root, "crates/iota-crypto", path))).digest("hex"), path).toBe(hash);
    }
    for (const [path, hash] of Object.entries(provenance.relocatedTestProductionHashes)) {
      const content = source(`crates/iota-crypto/${path}`).split(/#\[cfg\((?:all\()?test\b/)[0].trimEnd();
      expect(createHash("sha256").update(content).digest("hex"), path).toBe(hash);
    }
  });

  it("executes upstream curve vectors and entropy regressions in native CI", () => {
    expect(source(".github/workflows/native-ci.yml")).toContain("npm run test:stronghold-crypto");
    const runner = source("test/tooling/stronghold-crypto-check.mjs");
    expect(runner).toContain('"test", "--locked", "--manifest-path", "test/tooling/iota-crypto-check/Cargo.toml"');
    const harness = source("test/tooling/iota-crypto-check/Cargo.toml");
    for (const name of ["secp256k1_ecdsa", "slip10", "entropy_bridge"]) {
      expect(harness).toContain(`path = "../../rust/iota-crypto/${name}.rs"`);
    }
    expect(createHash("sha256").update(readFileSync(resolve(root, "test/fixtures/stronghold/upstream-2.1.0.snapshot"))).digest("hex"))
      .toBe("114586ada3dd289efa16d21f9e7a847bc882aedce98f5196659d77bace34163a");
  });
});
