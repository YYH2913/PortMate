import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { createUpdateManifest, mergeUpdateManifests } from "../../scripts/create-update-manifest.mjs";
import { verifyUpdateSignature } from "../../scripts/update-signature.mjs";

const fixture = new URL("../fixtures/updater/", import.meta.url);
const bytes = readFileSync(new URL("payload.txt", fixture));
const signature = readFileSync(new URL("payload.txt.sig", fixture), "utf8").trim();
const publicKey = readFileSync(new URL("public.key", fixture), "utf8").trim();

describe("signed update publication", () => {
  it("verifies real CLI signatures and rejects changed payloads, versions and trusted comments", () => {
    expect(() => verifyUpdateSignature(bytes, signature, publicKey, "0.2.0")).not.toThrow();
    expect(() => verifyUpdateSignature(Buffer.concat([bytes, Buffer.from("changed")]), signature, publicKey, "0.2.0")).toThrow();
    expect(() => verifyUpdateSignature(bytes, signature, publicKey, "0.3.0")).toThrow(/version/);
    const substituted = Buffer.from(Buffer.from(signature, "base64").toString().replace("version:0.2.0", "version:0.3.0")).toString("base64");
    expect(() => verifyUpdateSignature(bytes, substituted, publicKey, "0.3.0")).toThrow(/verification/);
  });

  it("keeps installer kinds separate and only publishes verified assets", () => {
    const root = mkdtempSync(join(tmpdir(), "portmate-manifest-"));
    try {
      const artifacts = ["PortMate_0.2.0_x64.msi", "PortMate_0.2.0_x64-setup.exe", "PortMate-windows-portable.zip"].map(name => join(root, name));
      for (const path of artifacts) writeFileSync(path, bytes);
      const input = { version: "0.2.0", tag: "v0.2.0", target: "windows-x86_64", notes: "English / 中文", publishedAt: "2026-10-20T00:00:00Z", artifacts, publicKey, sign: () => signature };
      const manifest = createUpdateManifest(input);
      expect(Object.keys(manifest.platforms)).toEqual(["windows-x86_64-msi", "windows-x86_64-nsis"]);
      expect(manifest.platforms["windows-x86_64-msi"].url).toBe("https://github.com/YYH2913/PortMate/releases/download/v0.2.0/PortMate_0.2.0_x64.msi");
      expect(() => createUpdateManifest({ ...input, artifacts: [...artifacts, artifacts[0]] })).toThrow(/Multiple/);
      expect(() => createUpdateManifest({ ...input, version: "0.3.0", tag: "0.3.0" })).toThrow(/version/);
      expect(() => createUpdateManifest({ ...input, tag: "wrong" })).toThrow(/matching tag/);
      writeFileSync(artifacts[0], "tampered");
      expect(() => createUpdateManifest(input)).toThrow(/verification/);
    } finally { rmSync(root, { recursive: true, force: true }); }
  });

  it("merges matching platform fragments and rejects conflicting release identities or duplicate targets", () => {
    const base = { version: "0.2.0", notes: "English / 中文", pub_date: "2026-10-20T00:00:00.000Z" };
    const windows = { ...base, platforms: { "windows-x86_64-msi": { url: "windows", signature } } };
    const linux = { ...base, platforms: { "linux-x86_64-appimage": { url: "linux", signature } } };
    expect(Object.keys(mergeUpdateManifests([windows, linux]).platforms)).toHaveLength(2);
    expect(() => mergeUpdateManifests([windows, windows])).toThrow(/Duplicate/);
    expect(() => mergeUpdateManifests([windows, { ...linux, notes: "other" }])).toThrow(/identities/);
  });
});
