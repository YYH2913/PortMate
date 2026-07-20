import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { prepareAppImageBuildEnvironment } from "./appimage-runtime.mjs";

const temporaryRoots = [];

afterEach(() => {
  for (const path of temporaryRoots.splice(0)) {
    rmSync(path, { recursive: true, force: true });
  }
});

describe("prepareAppImageBuildEnvironment", () => {
  it("preserves a valid caller-provided runtime", () => {
    const root = temporaryRoot();
    const runtime = join(root, "runtime");
    writeFileSync(runtime, "runtime");

    const prepared = prepareAppImageBuildEnvironment(
      { PATH: process.env.PATH, LDAI_RUNTIME_FILE: runtime },
      { platform: "linux", tempRoot: root },
    );

    expect(prepared.source).toBe("environment");
    expect(prepared.env.LDAI_RUNTIME_FILE).toBe(runtime);
    prepared.cleanup();
    expect(readFileSync(runtime, "utf8")).toBe("runtime");
  });

  it("extracts the runtime prefix from the cached Tauri plugin", () => {
    const root = temporaryRoot();
    const home = join(root, "home");
    const plugin = join(home, ".cache", "tauri", "linuxdeploy-plugin-appimage.AppImage");
    mkdirSync(dirname(plugin), { recursive: true });
    const image = fakeType2AppImage(128, 256);
    writeFileSync(plugin, image);

    const prepared = prepareAppImageBuildEnvironment(
      { PATH: process.env.PATH },
      { platform: "linux", home, tempRoot: root },
    );

    expect(prepared.source).toBe("tauri-cache");
    expect(readFileSync(prepared.runtimeFile)).toEqual(image.subarray(0, 128));
    const runtimeFile = prepared.runtimeFile;
    prepared.cleanup();
    expect(existsSync(runtimeFile)).toBe(false);
  });

  it("allows linuxdeploy to use its normal fallback when no cache exists", () => {
    const root = temporaryRoot();
    const prepared = prepareAppImageBuildEnvironment(
      { PATH: process.env.PATH },
      { platform: "linux", home: join(root, "missing-home"), tempRoot: root },
    );

    expect(prepared.source).toBeNull();
    expect(prepared.runtimeFile).toBeNull();
    expect(prepared.env).not.toHaveProperty("LDAI_RUNTIME_FILE");
  });

  it("rejects an invalid caller-provided runtime instead of hiding the configuration error", () => {
    const root = temporaryRoot();

    expect(() => prepareAppImageBuildEnvironment(
      { LDAI_RUNTIME_FILE: join(root, "missing") },
      { platform: "linux", tempRoot: root },
    )).toThrow(/Configured AppImage runtime is unavailable/);
  });
});

function temporaryRoot() {
  const root = mkdtempSync(join(tmpdir(), "portmate-appimage-runtime-test-"));
  temporaryRoots.push(root);
  return root;
}

function fakeType2AppImage(runtimeBytes, imageBytes) {
  const image = Buffer.alloc(imageBytes);
  image.set([0x7f, 0x45, 0x4c, 0x46], 0);
  image.set([0x41, 0x49, 0x02], 8);
  image.write("hsqs", 32, "ascii");
  const superblock = image.subarray(runtimeBytes, runtimeBytes + 96);
  superblock.write("hsqs", 0, "ascii");
  superblock.writeUInt32LE(1, 4);
  superblock.writeUInt32LE(128 * 1024, 12);
  superblock.writeUInt16LE(1, 20);
  superblock.writeUInt16LE(17, 22);
  superblock.writeUInt16LE(1, 26);
  superblock.writeUInt16LE(4, 28);
  superblock.writeBigUInt64LE(BigInt(imageBytes - runtimeBytes), 40);
  superblock.writeBigUInt64LE(100n, 48);
  superblock.writeBigUInt64LE(0xffff_ffff_ffff_ffffn, 56);
  superblock.writeBigUInt64LE(96n, 64);
  superblock.writeBigUInt64LE(104n, 72);
  superblock.writeBigUInt64LE(0xffff_ffff_ffff_ffffn, 80);
  superblock.writeBigUInt64LE(0xffff_ffff_ffff_ffffn, 88);
  return image;
}
