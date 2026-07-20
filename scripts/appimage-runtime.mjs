import {
  closeSync,
  existsSync,
  lstatSync,
  mkdtempSync,
  mkdirSync,
  openSync,
  readSync,
  rmSync,
  statSync,
  writeSync,
} from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join, resolve } from "node:path";

const appImageMarker = Buffer.from([0x7f, 0x45, 0x4c, 0x46, 0, 0, 0, 0, 0x41, 0x49, 0x02]);
const squashfsMagic = Buffer.from("hsqs", "ascii");
const squashfsSuperblockBytes = 96;
const unsignedLongLongMax = 0xffff_ffff_ffff_ffffn;

export function cachedAppImagePluginPath(env = process.env, home = homedir()) {
  return env.PORTMATE_APPIMAGE_PLUGIN?.trim()
    ? resolve(env.PORTMATE_APPIMAGE_PLUGIN)
    : join(home, ".cache", "tauri", "linuxdeploy-plugin-appimage.AppImage");
}

export function prepareAppImageBuildEnvironment(sourceEnv = process.env, options = {}) {
  const platform = options.platform ?? process.platform;
  const env = { ...sourceEnv };
  const unchanged = { env, runtimeFile: null, source: null, cleanup() {} };
  if (platform !== "linux") return unchanged;

  const configuredRuntime = sourceEnv.LDAI_RUNTIME_FILE?.trim();
  if (configuredRuntime) {
    const runtimeFile = resolve(configuredRuntime);
    assertRegularNonEmptyFile(runtimeFile, "Configured AppImage runtime");
    env.LDAI_RUNTIME_FILE = runtimeFile;
    return { ...unchanged, env, runtimeFile, source: "environment" };
  }

  const plugin = cachedAppImagePluginPath(sourceEnv, options.home ?? homedir());
  if (!existsSync(plugin)) return unchanged;
  assertRegularNonEmptyFile(plugin, "Cached Tauri AppImage plugin");

  const tempRoot = options.tempRoot ?? tmpdir();
  mkdirSync(tempRoot, { recursive: true });
  const workRoot = mkdtempSync(join(tempRoot, "portmate-appimage-runtime-"));
  const runtimeFile = join(workRoot, "runtime");
  try {
    copyAppImageRuntime(plugin, runtimeFile, options.runtimeBytes);
  } catch (error) {
    rmSync(workRoot, { recursive: true, force: true });
    throw error;
  }
  env.LDAI_RUNTIME_FILE = runtimeFile;
  return {
    env,
    runtimeFile,
    source: "tauri-cache",
    cleanup() {
      rmSync(workRoot, { recursive: true, force: true });
    },
  };
}

export function copyAppImageRuntime(appImage, destination, knownRuntimeBytes) {
  const runtimeBytes = knownRuntimeBytes ?? findAppImageRuntimeSize(appImage);
  if (!Number.isSafeInteger(runtimeBytes) || runtimeBytes <= 0) {
    throw new Error(`Invalid known AppImage runtime size: ${runtimeBytes}`);
  }

  const metadata = statSync(appImage);
  if (!metadata.isFile() || metadata.size < runtimeBytes) {
    throw new Error(
      `AppImage runtime size ${runtimeBytes} exceeds the ${metadata.size}-byte source: ${appImage}`,
    );
  }

  let source;
  let target;
  const buffer = Buffer.allocUnsafe(64 * 1024);
  let offset = 0;
  try {
    source = openSync(appImage, "r");
    target = openSync(destination, "wx", 0o644);
    while (offset < runtimeBytes) {
      const requested = Math.min(buffer.length, runtimeBytes - offset);
      const bytesRead = readSync(source, buffer, 0, requested, offset);
      if (bytesRead === 0) throw new Error(`AppImage ended before its ${runtimeBytes}-byte runtime`);
      let written = 0;
      while (written < bytesRead) {
        const bytesWritten = writeSync(target, buffer, written, bytesRead - written);
        if (bytesWritten === 0) throw new Error("Unable to write the copied AppImage runtime");
        written += bytesWritten;
      }
      offset += bytesRead;
    }
  } finally {
    if (source !== undefined) closeSync(source);
    if (target !== undefined) closeSync(target);
  }
}

export function findAppImageRuntimeSize(appImage) {
  const metadata = statSync(appImage);
  if (!metadata.isFile() || metadata.size < squashfsSuperblockBytes) {
    throw new Error(`AppImage must be a regular type-2 image: ${appImage}`);
  }

  let source;
  try {
    source = openSync(appImage, "r");
    const header = Buffer.alloc(appImageMarker.length);
    readExact(source, header, 0);
    if (
      !header.subarray(0, 4).equals(appImageMarker.subarray(0, 4))
      || !header.subarray(8).equals(appImageMarker.subarray(8))
    ) {
      throw new Error(`AppImage does not contain a type-2 ELF marker: ${appImage}`);
    }

    const chunk = Buffer.allocUnsafe(256 * 1024);
    let carry = Buffer.alloc(0);
    let position = 0;
    while (position < metadata.size) {
      const bytesRead = readSync(
        source,
        chunk,
        0,
        Math.min(chunk.length, metadata.size - position),
        position,
      );
      if (bytesRead === 0) break;
      const searchable = Buffer.concat([carry, chunk.subarray(0, bytesRead)]);
      const searchableOffset = position - carry.length;
      let index = searchable.indexOf(squashfsMagic);
      while (index !== -1) {
        const candidate = searchableOffset + index;
        if (candidate > 0 && isSquashfsSuperblock(source, candidate, metadata.size)) {
          return candidate;
        }
        index = searchable.indexOf(squashfsMagic, index + 1);
      }
      carry = Buffer.from(searchable.subarray(Math.max(0, searchable.length - 3)));
      position += bytesRead;
    }
  } finally {
    if (source !== undefined) closeSync(source);
  }
  throw new Error(`Unable to locate a valid SquashFS payload in AppImage: ${appImage}`);
}

function isSquashfsSuperblock(source, offset, fileSize) {
  if (offset + squashfsSuperblockBytes > fileSize) return false;
  const block = Buffer.allocUnsafe(squashfsSuperblockBytes);
  try {
    readExact(source, block, offset);
  } catch {
    return false;
  }

  const blockSize = block.readUInt32LE(12);
  const blockLog = block.readUInt16LE(22);
  const bytesUsed = block.readBigUInt64LE(40);
  if (
    !block.subarray(0, 4).equals(squashfsMagic)
    || block.readUInt32LE(4) === 0
    || blockSize < 4096
    || blockSize > 1024 * 1024
    || (blockSize & (blockSize - 1)) !== 0
    || blockLog !== Math.log2(blockSize)
    || block.readUInt16LE(20) === 0
    || block.readUInt16LE(26) === 0
    || block.readUInt16LE(28) !== 4
    || block.readUInt16LE(30) !== 0
    || bytesUsed < BigInt(squashfsSuperblockBytes)
    || bytesUsed > BigInt(fileSize - offset)
  ) {
    return false;
  }

  for (const tableOffset of [48, 64, 72]) {
    const table = block.readBigUInt64LE(tableOffset);
    if (table === unsignedLongLongMax || table >= bytesUsed) return false;
  }
  for (const tableOffset of [56, 80, 88]) {
    const table = block.readBigUInt64LE(tableOffset);
    if (table !== unsignedLongLongMax && table >= bytesUsed) return false;
  }
  return true;
}

function readExact(source, buffer, position) {
  let offset = 0;
  while (offset < buffer.length) {
    const bytesRead = readSync(source, buffer, offset, buffer.length - offset, position + offset);
    if (bytesRead === 0) throw new Error(`Unexpected end of file at byte ${position + offset}`);
    offset += bytesRead;
  }
}

function assertRegularNonEmptyFile(path, label) {
  let metadata;
  try {
    metadata = lstatSync(path);
  } catch (error) {
    throw new Error(`${label} is unavailable: ${path}`, { cause: error });
  }
  if (!metadata.isFile() || metadata.size === 0) {
    throw new Error(`${label} must be a non-empty regular file: ${path}`);
  }
}
