import { createHash } from "node:crypto";
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, renameSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { verifyWindowsPackageLayout } from "./native-package-layout.mjs";
import { verifyWindowsReleaseBinary, verifyWindowsSidecarBinary } from "./windows-release-binary.mjs";
import { windowsBundleReference } from "./windows-bundle-reference.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const target = resolve(root, process.env.CARGO_TARGET_DIR || "target");
const release = join(target, "release");
const bundle = join(release, "bundle");
const version = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;
const workRoot = join(target, "native-release-archives");
mkdirSync(workRoot, { recursive: true });
const work = mkdtempSync(join(workRoot, "archive-"));
const artifacts = [];
let windows;

if (process.platform === "win32" && process.arch === "x64") {
  const name = `PortMate-${version}-windows-x86_64`;
  const stage = join(work, name);
  mkdirSync(join(stage, "THIRD_PARTY_LICENSES"), { recursive: true });
  const source = readFileSync(join(release, "portmate.exe"));
  const prepared = windowsBundleReference(source, "portable").bytes;
  const checksumOffset = prepared.readUInt32LE(0x3c) + 24 + 64;
  if (prepared.readUInt32LE(checksumOffset) !== 0) throw new Error("Native MSVC portable builds require the standard zero PE checksum");
  writeFileSync(join(stage, "portmate.exe"), prepared);
  copyFileSync(join(release, "portmate-mcp.exe"), join(stage, "portmate-mcp.exe"));
  copyFileSync(join(root, "LICENSE"), join(stage, "LICENSE"));
  copyFileSync(join(root, "THIRD_PARTY_LICENSES/JetBrainsMono-OFL.txt"), join(stage, "THIRD_PARTY_LICENSES/JetBrainsMono-OFL.txt"));
  if (existsSync(join(release, "WebView2Loader.dll"))) copyFileSync(join(release, "WebView2Loader.dll"), join(stage, "WebView2Loader.dll"));
  verifyWindowsReleaseBinary({ executable: join(stage, "portmate.exe"), frontendDist: join(root, "dist") });
  verifyWindowsSidecarBinary(join(stage, "portmate-mcp.exe"));
  windows = verifyWindowsPackageLayout({ root: stage, sourceMain: join(stage, "portmate.exe"), sourceSidecar: join(release, "portmate-mcp.exe"), sourceLicense: join(root, "LICENSE"), sourceThirdPartyLicense: join(root, "THIRD_PARTY_LICENSES/JetBrainsMono-OFL.txt") });
  const folder = join(bundle, "portable");
  mkdirSync(folder, { recursive: true });
  const archive = join(folder, `${name}-portable.zip`);
  run("tar.exe", ["-a", "-c", "-f", archive, "-C", work, name]);
  const extracted = join(work, "extracted");
  mkdirSync(extracted);
  run("tar.exe", ["-x", "-f", archive, "-C", extracted]);
  verifyWindowsPackageLayout({ root: extracted, sourceMain: windows.main, sourceSidecar: windows.sidecar, sourceLicense: windows.license, sourceThirdPartyLicense: windows.thirdPartyLicense });
  artifacts.push(archive);
} else if (process.platform === "darwin" && process.arch === "arm64") {
  const folder = join(bundle, "macos");
  const app = join(folder, "PortMate.app");
  const updater = join(folder, `PortMate_${version}_aarch64.app.tar.gz`);
  const portable = join(folder, `PortMate-${version}-macos-aarch64-portable.zip`);
  run("tar", ["-czf", updater, "-C", folder, "PortMate.app"]);
  run("ditto", ["-c", "-k", "--sequesterRsrc", "--keepParent", app, portable]);
  artifacts.push(updater, portable);
} else if (process.platform === "linux" && process.arch === "x64") {
  const image = join(bundle, "appimage", `PortMate_${version}_amd64.AppImage`);
  chmodSync(image, 0o755);
  run(image, ["--appimage-extract"], work);
  const name = `PortMate-${version}-linux-x86_64`;
  renameSync(join(work, "squashfs-root"), join(work, name));
  const folder = join(bundle, "portable");
  mkdirSync(folder, { recursive: true });
  const portable = join(folder, `${name}-portable.tar.gz`);
  run("tar", ["-czf", portable, "-C", work, name]);
  artifacts.push(portable);
} else {
  throw new Error(`Release archives are not configured for ${process.platform}/${process.arch}`);
}

const record = {
  version, platform: process.platform, architecture: process.arch,
  sourceCommit: run("git", ["rev-parse", "HEAD"]).trim(),
  artifacts: artifacts.map((path) => ({ path, bytes: statSync(path).size, sha256: createHash("sha256").update(readFileSync(path)).digest("hex") })),
  ...(windows ? { windows } : {}),
};
mkdirSync(join(target, "native-ci"), { recursive: true });
writeFileSync(join(target, "native-ci/release-archives.json"), JSON.stringify(record, null, 2) + "\n");
console.log(JSON.stringify(record, null, 2));

function run(command, args, cwd = root) {
  const result = spawnSync(command, args, { cwd, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} failed (${result.status}): ${result.stderr?.slice(-4_000)}`);
  return result.stdout;
}
