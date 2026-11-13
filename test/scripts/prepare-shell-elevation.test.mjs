import { createHash } from "node:crypto";
import { mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { elevationManifest, prepareElevationHelper, verifyElevationArchive } from "../../scripts/prepare-shell-elevation.mjs";

const roots = [];
const originalHash = elevationManifest.sha256;
afterEach(async () => {
  elevationManifest.sha256 = originalHash;
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
});
async function cacheDirectory() {
  const root = await mkdtemp(join(tmpdir(), "portmate-elevation-")); roots.push(root); return root;
}

describe("bundled Windows shell elevation preparation", () => {
  it("pins the upstream release and selects only supported Windows executables", async () => {
    expect(elevationManifest.version).toBe("2.6.1");
    expect(elevationManifest.url).toBe("https://github.com/gerardog/gsudo/releases/download/v2.6.1/gsudo.portable.zip");
    expect(elevationManifest.sha256).toBe("21130bf178d7b9891207f00bff56f05b6b363ce9cec7b2d084e45fb12ee51f44");
    expect(elevationManifest.targets["x86_64-pc-windows-gnu"]).toBe("x64/gsudo.exe");
    expect(elevationManifest.targets["aarch64-pc-windows-msvc"]).toBe("arm64/gsudo.exe");
    const network = vi.fn();
    for (const target of ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"]) {
      expect(await prepareElevationHelper({ target, fetchArchive: network })).toBe(null);
    }
    expect(network).not.toHaveBeenCalled();
    await expect(prepareElevationHelper({ target: "i686-pc-windows-msvc", fetchArchive: network })).rejects.toThrow("Unsupported Windows");
    expect(network).not.toHaveBeenCalled();
  });

  it("verifies before publishing the cache and supports offline rebuilds", async () => {
    const cache = await cacheDirectory();
    const bytes = Buffer.from("fixture archive");
    elevationManifest.sha256 = createHash("sha256").update(bytes).digest("hex");
    const network = vi.fn(async () => new Response(bytes));
    const destination = await prepareElevationHelper({ target: "x86_64-pc-windows-msvc", cache, fetchArchive: network });
    expect(await readFile(destination)).toEqual(bytes);
    expect(network).toHaveBeenCalledOnce();
    const offline = vi.fn(() => { throw Error("offline"); });
    expect(await prepareElevationHelper({ target: "x86_64-pc-windows-gnu", cache, fetchArchive: offline })).toBe(destination);
    expect(offline).not.toHaveBeenCalled();
    const corrupt = join(cache, "corrupt.zip"); await writeFile(corrupt, "tampered");
    await expect(prepareElevationHelper({ target: "x86_64-pc-windows-msvc", cache, archive: corrupt })).rejects.toThrow("integrity");
    expect(await readFile(destination)).toEqual(bytes);
    expect(await readdir(cache)).toEqual(["corrupt.zip", "gsudo.portable.zip"]);
    await writeFile(destination, "corrupt cache");
    await prepareElevationHelper({ target: "x86_64-pc-windows-msvc", cache, fetchArchive: network });
    expect(await readFile(destination)).toEqual(bytes);
    expect(network).toHaveBeenCalledTimes(2);
  });

  it("rejects failed, oversized and corrupt downloads without creating a partial cache", async () => {
    const cache = await cacheDirectory();
    const options = { target: "x86_64-pc-windows-msvc", cache };
    await expect(prepareElevationHelper({ ...options, fetchArchive: async () => new Response(null, { status: 403 }) })).rejects.toThrow("HTTP 403");
    await expect(prepareElevationHelper({ ...options, fetchArchive: async () => new Response("bad") })).rejects.toThrow("integrity");
    await expect(prepareElevationHelper({ ...options, fetchArchive: async () => new Response("x", { headers: { "content-length": String(17 * 1024 * 1024) } }) })).rejects.toThrow("16 MiB");
    const body = new ReadableStream({ start(controller) { controller.enqueue(new Uint8Array(17 * 1024 * 1024)); controller.close(); } });
    await expect(prepareElevationHelper({ ...options, fetchArchive: async () => new Response(body) })).rejects.toThrow("16 MiB");
    expect(await readdir(cache)).toEqual([]);
    expect(() => verifyElevationArchive(Buffer.alloc(0))).toThrow("integrity");
    expect(() => verifyElevationArchive(Buffer.alloc(17 * 1024 * 1024))).toThrow("integrity");
  });
});
