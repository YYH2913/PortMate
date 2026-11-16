import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";

const source = path => readFileSync(new URL(`../../${path}`, import.meta.url), "utf8");
const sha = path => createHash("sha256").update(readFileSync(new URL(`../../${path}`, import.meta.url))).digest("hex");

describe("native PTY initialization and attribution", () => {
  it("pins the upstream package and preserves all unmodified source bytes", () => {
    const provenance = JSON.parse(source("crates/portable-pty/PORTMATE_UPSTREAM.json"));
    expect(provenance.crateSha256).toBe("b4a596a2b3d2752d94f51fac2d4a96737b8705dddd311a32b9af47211f08671e");
    expect(source("Cargo.toml")).toContain('portable-pty = { path = "crates/portable-pty" }');
    expect(source(".gitattributes")).toMatch(/^crates\/portable-pty\/\*\* text eol=lf$/m);
    const locked = source("Cargo.lock").split('name = "portable-pty"')[1].split("[[package]]")[0];
    expect(locked).toContain('version = "0.9.0"');
    expect(locked).not.toContain("registry+");
    for (const [path, hash] of Object.entries(provenance.unchangedSources)) expect(sha(`crates/portable-pty/${path}`), path).toBe(hash);
    expect(source("crates/portable-pty/src/cmdbuilder.rs")).toContain('../../../test/rust/portable-pty/cmdbuilder.rs');
    expect(source("test/rust/portable-pty/cmdbuilder.rs")).toContain("#[test]");
  });
  it("does not require frontend cursor replies to initialize a blank Windows terminal", () => {
    const windows = source("crates/portable-pty/src/win/psuedocon.rs");
    expect(windows).not.toContain("PSUEDOCONSOLE_INHERIT_CURSOR");
    expect(windows).toContain("PSEUDOCONSOLE_RESIZE_QUIRK | PSEUDOCONSOLE_WIN32_INPUT_MODE,");
    expect(windows).toContain("https://learn.microsoft.com/windows/console/createpseudoconsole");
    const config = JSON.parse(source("src-tauri/tauri.conf.json"));
    expect(config.bundle.resources["../licenses/portable-pty-MIT.txt"]).toBe("THIRD_PARTY_LICENSES/portable-pty-MIT.txt");
    expect(source("licenses/portable-pty-MIT.txt")).toBe(source("crates/portable-pty/LICENSE.md"));
    expect(source("licenses/portable-pty-MIT.txt")).toContain("Copyright (c) 2018 Wez Furlong");
  });
});
