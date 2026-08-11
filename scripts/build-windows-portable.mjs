import { createHash } from "node:crypto";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  rmSync,
} from "node:fs";
import { delimiter, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { verifyWindowsPackageLayout } from "./native-package-layout.mjs";
import {
  verifyWindowsLoaderDll,
  verifyWindowsReleaseBinary,
  verifyWindowsSidecarBinary,
} from "./windows-release-binary.mjs";

const target = "x86_64-pc-windows-gnu";
const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const tauriConfig = JSON.parse(readFileSync(join(projectRoot, "src-tauri", "tauri.conf.json"), "utf8"));
const targetDirectory = resolveTargetDirectory();
const releaseRoot = join(targetDirectory, target, "release");
const main = join(releaseRoot, "portmate.exe");
const sidecar = join(releaseRoot, "portmate-mcp.exe");
const loader = join(releaseRoot, "WebView2Loader.dll");
const license = join(projectRoot, "LICENSE");
const thirdPartyLicense = join(projectRoot, "THIRD_PARTY_LICENSES", "JetBrainsMono-OFL.txt");
const artifactRoot = join(targetDirectory, "release-artifacts");
const artifactName = `PortMate-${tauriConfig.version}-windows-x86_64`;
const stagingRoot = join(artifactRoot, artifactName);
const archive = join(artifactRoot, `${artifactName}-portable.zip`);
const env = windowsGnuBuildEnvironment();

run(
  process.execPath,
  [
    join(projectRoot, "node_modules", "@tauri-apps", "cli", "tauri.js"),
    "build",
    "--target",
    target,
    "--no-bundle",
    "--",
    "--locked",
  ],
  { env },
);

const mainVerification = verifyWindowsReleaseBinary({
  executable: main,
  frontendDist: join(projectRoot, "dist"),
});
const sidecarVerification = verifyWindowsSidecarBinary(sidecar);
const loaderVerification = verifyWindowsLoaderDll(loader);

mkdirSync(artifactRoot, { recursive: true });
removeGeneratedArtifact(stagingRoot);
removeGeneratedArtifact(archive);
mkdirSync(join(stagingRoot, "THIRD_PARTY_LICENSES"), { recursive: true });
copy(main, join(stagingRoot, "portmate.exe"), 0o755);
copy(sidecar, join(stagingRoot, "portmate-mcp.exe"), 0o755);
copy(loader, join(stagingRoot, "WebView2Loader.dll"), 0o644);
copy(license, join(stagingRoot, "LICENSE"), 0o644);
copy(thirdPartyLicense, join(stagingRoot, "THIRD_PARTY_LICENSES", "JetBrainsMono-OFL.txt"), 0o644);

const payload = verifyWindowsPackageLayout({
  root: stagingRoot,
  sourceMain: main,
  sourceSidecar: sidecar,
  sourceLicense: license,
  sourceThirdPartyLicense: thirdPartyLicense,
});
run("zip", ["-q", "-9", "-r", archive, artifactName], { cwd: artifactRoot, env });
run("zip", ["-T", archive], { cwd: artifactRoot, env });

console.log(JSON.stringify({
  target,
  archive,
  archiveBytes: lstatSync(archive).size,
  archiveSha256: sha256(archive),
  main: mainVerification,
  sidecar: sidecarVerification,
  webView2Loader: loaderVerification,
  payload,
}, null, 2));

function windowsGnuBuildEnvironment() {
  const next = { ...process.env };
  const localBin = join(targetDirectory, "mingw-toolchain", "root", "usr", "bin");
  if (existsSync(localBin)) {
    next.PATH = `${localBin}${delimiter}${next.PATH || ""}`;
  }

  const compiler = next.CC_x86_64_pc_windows_gnu
    || firstAvailableCommand([
      join(localBin, "x86_64-w64-mingw32-gcc-posix"),
      "x86_64-w64-mingw32-gcc-posix",
      "x86_64-w64-mingw32-gcc",
    ], next);
  if (!compiler) {
    throw new Error(
      "Windows GNU compiler is unavailable; set CC_x86_64_pc_windows_gnu or install MinGW-w64",
    );
  }
  const archiver = next.AR_x86_64_pc_windows_gnu
    || firstAvailableCommand([
      join(localBin, "x86_64-w64-mingw32-ar"),
      "x86_64-w64-mingw32-ar",
    ], next);
  if (!archiver) {
    throw new Error(
      "Windows GNU archiver is unavailable; set AR_x86_64_pc_windows_gnu or install MinGW-w64",
    );
  }

  next.CC_x86_64_pc_windows_gnu = compiler;
  next.AR_x86_64_pc_windows_gnu = archiver;
  next.CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER
    ||= compiler;
  next.CARGO_BUILD_TARGET = target;

  const localSodium = join(targetDirectory, "mingw-libsodium", "install", "lib");
  if (!next.SODIUM_LIB_DIR && existsSync(join(localSodium, "libsodium.a"))) {
    next.SODIUM_LIB_DIR = localSodium;
  }
  if (!next.SODIUM_LIB_DIR || !existsSync(join(next.SODIUM_LIB_DIR, "libsodium.a"))) {
    throw new Error(
      "Windows GNU libsodium.a is unavailable; set SODIUM_LIB_DIR to its library directory",
    );
  }
  return next;
}

function resolveTargetDirectory() {
  const configured = process.env.CARGO_TARGET_DIR;
  if (!configured) return join(projectRoot, "target");
  return isAbsolute(configured) ? configured : resolve(projectRoot, configured);
}

function firstAvailableCommand(candidates, env) {
  for (const candidate of candidates) {
    if (isAbsolute(candidate) && !existsSync(candidate)) continue;
    const result = spawnSync(candidate, ["--version"], { env, stdio: "ignore" });
    if (!result.error && result.status === 0) return candidate;
  }
  return null;
}

function copy(source, destination, mode) {
  if (!lstatSync(source).isFile()) {
    throw new Error(`Portable Windows payload source is not a regular file: ${source}`);
  }
  copyFileSync(source, destination);
  if (process.platform !== "win32") chmodSync(destination, mode);
}

function removeGeneratedArtifact(path) {
  const resolved = resolve(path);
  const relativePath = relative(artifactRoot, resolved);
  if (!relativePath || isAbsolute(relativePath)
      || relativePath === ".." || relativePath.startsWith(`..${sep}`)) {
    throw new Error(`Refusing to remove a path outside the release artifact root: ${resolved}`);
  }
  rmSync(resolved, { recursive: true, force: true });
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd || projectRoot,
    env: options.env || process.env,
    stdio: "inherit",
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} failed with exit code ${result.status ?? 1}`);
  }
}
