import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { easyTierManifest, easyTierRelease, prepareEasyTier, verifyEasyTierArchive } from "../../scripts/prepare-easytier.mjs";

const roots = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
});

describe("bundled EasyTier preparation", () => {
  it("selects the target architecture, including Windows cross builds", () => {
    expect(easyTierRelease("x86_64-pc-windows-gnu")).toEqual(easyTierRelease("x86_64-pc-windows-msvc"));
    expect(easyTierRelease("aarch64-apple-darwin").archive).toContain("macos-aarch64");
    expect(easyTierRelease("x86_64-unknown-linux-gnu").files).toEqual(["easytier-core", "easytier-cli"]);
    expect(() => easyTierRelease("aarch64-unknown-linux-gnu")).toThrow("does not support");
    expect(easyTierRelease("x86_64-pc-windows-gnu").files).toContain("wintun.dll");
  });

  it("verifies archives before caching and never replaces a valid cache with corrupt data", async () => {
    const cache = await mkdtemp(join(tmpdir(), "portmate-builtin-easytier-")); roots.push(cache);
    const bytes = Buffer.from("fixture upstream archive");
    const release = easyTierManifest.releases["linux-x86_64"];
    const originalHash = release.sha256;
    release.sha256 = createHash("sha256").update(bytes).digest("hex");
    const destination = join(cache, release.archive);
    try {
      const network = vi.fn(async () => new Response(bytes));
      await prepareEasyTier({ target: "x86_64-unknown-linux-gnu", cache, fetchArchive: network });
      expect(network).toHaveBeenCalledOnce();
      const offline = vi.fn(() => { throw Error("offline"); });
      await prepareEasyTier({ target: "x86_64-unknown-linux-gnu", cache, fetchArchive: offline });
      expect(offline).not.toHaveBeenCalled();
      const badArchive = join(cache, "bad.zip"); await writeFile(badArchive, "modified upstream archive");
      await expect(prepareEasyTier({ target: "x86_64-unknown-linux-gnu", cache, archive: badArchive })).rejects.toThrow("integrity");
      expect(await readFile(destination)).toEqual(bytes);
      await writeFile(destination, "corrupt cache");
      await expect(prepareEasyTier({ target: "x86_64-unknown-linux-gnu", cache, fetchArchive: async () => new Response("bad") })).rejects.toThrow("integrity");
      expect(await readFile(destination, "utf8")).toBe("corrupt cache");
      await prepareEasyTier({ target: "x86_64-unknown-linux-gnu", cache, fetchArchive: network });
      expect(await readFile(destination)).toEqual(bytes);
    } finally { release.sha256 = originalHash; }
  });

  it("rejects mismatched and oversized archives and does not accept HTTP failures", async () => {
    expect(() => verifyEasyTierArchive(Buffer.from("tampered"), easyTierRelease("x86_64-unknown-linux-gnu"))).toThrow("integrity");
    const cache = await mkdtemp(join(tmpdir(), "portmate-builtin-easytier-")); roots.push(cache);
    await expect(prepareEasyTier({ target: "x86_64-unknown-linux-gnu", cache, fetchArchive: async () => new Response(null, { status: 404 }) })).rejects.toThrow("HTTP 404");
    await expect(prepareEasyTier({ target: "x86_64-unknown-linux-gnu", cache, fetchArchive: async () => new Response("x", { headers: { "content-length": String(65 * 1024 * 1024) } }) })).rejects.toThrow("64 MiB");
  });
});
