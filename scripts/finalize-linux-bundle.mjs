import {
  chmodSync,
  closeSync,
  existsSync,
  lstatSync,
  mkdtempSync,
  mkdirSync,
  openSync,
  readFileSync,
  readSync,
  readdirSync,
  readlinkSync,
  renameSync,
  rmSync,
  symlinkSync,
  unlinkSync,
  writeSync,
} from "node:fs";
import { homedir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

if (process.platform === "linux") finalizeLinuxAppImage();

function finalizeLinuxAppImage() {
  const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const { version } = JSON.parse(readFileSync(join(projectRoot, "package.json"), "utf8"));
  const packageArchitecture = process.arch === "x64" ? "amd64" : process.arch;
  const appImage = join(
    projectRoot,
    "target",
    "release",
    "bundle",
    "appimage",
    `PortMate_${version}_${packageArchitecture}.AppImage`,
  );
  const plugin = process.env.PORTMATE_APPIMAGE_PLUGIN
    ? resolve(process.env.PORTMATE_APPIMAGE_PLUGIN)
    : join(homedir(), ".cache", "tauri", "linuxdeploy-plugin-appimage.AppImage");
  if (!existsSync(appImage)) throw new Error(`AppImage was not produced: ${appImage}`);
  if (!existsSync(plugin)) throw new Error(`Tauri AppImage plugin is unavailable: ${plugin}`);

  const workRoot = mkdtempSync(join(dirname(appImage), ".portmate-finalize-"));
  try {
    run(appImage, ["--appimage-extract"], workRoot, true);
    const appDir = join(workRoot, "squashfs-root");
    normalizeTreePermissions(appDir);
    replaceSymlink(join(appDir, ".DirIcon"), "PortMate.png");
    const runtime = join(workRoot, "runtime");
    copyAppImageRuntime(appImage, runtime);
    const pluginRoot = join(workRoot, "plugin");
    mkdirSync(pluginRoot);
    run(plugin, ["--appimage-extract"], pluginRoot, true);
    const appImageTool = join(pluginRoot, "squashfs-root", "usr", "bin", "appimagetool");
    const rebuilt = join(workRoot, "PortMate.finalized.AppImage");
    run(appImageTool, ["--runtime-file", runtime, appDir, rebuilt], workRoot, true);
    if (!existsSync(rebuilt)) throw new Error(`AppImage plugin did not produce ${rebuilt}`);
    chmodSync(rebuilt, 0o755);
    renameSync(rebuilt, appImage);
    console.log(`Finalized portable AppImage metadata and permissions: ${appImage}`);
  } finally {
    rmSync(workRoot, { recursive: true, force: true });
  }
}

function copyAppImageRuntime(appImage, destination) {
  const result = spawnSync(appImage, ["--appimage-offset"], {
    encoding: "utf8",
    maxBuffer: 1024,
  });
  if (result.error) throw result.error;
  const runtimeBytes = Number(result.stdout?.trim());
  if (result.status !== 0 || !Number.isSafeInteger(runtimeBytes) || runtimeBytes <= 0) {
    throw new Error(`Unable to determine AppImage runtime size: ${result.stderr || result.stdout}`);
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

function normalizeTreePermissions(path) {
  const metadata = lstatSync(path);
  if (metadata.isSymbolicLink()) return;
  if (metadata.isDirectory()) {
    chmodSync(path, 0o755);
    for (const entry of readdirSync(path)) normalizeTreePermissions(join(path, entry));
    return;
  }
  if (metadata.isFile()) chmodSync(path, metadata.mode & 0o111 ? 0o755 : 0o644);
}

function replaceSymlink(path, target) {
  const metadata = lstatSync(path);
  if (!metadata.isSymbolicLink()) {
    throw new Error(`Expected ${path} to be a symbolic link`);
  }
  if (readlinkSync(path) === target) return;
  unlinkSync(path);
  symlinkSync(target, path);
}

function run(command, args, cwd, quiet) {
  const result = spawnSync(command, args, {
    cwd,
    env: process.env,
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
  });
  if (!quiet && result.stdout) process.stdout.write(result.stdout);
  if ((!quiet || result.status !== 0) && result.stderr) process.stderr.write(result.stderr);
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${basename(command)} ${args.join(" ")} failed with exit code ${result.status}`);
  }
}
