import { accessSync, constants, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

if (process.platform !== "linux") {
  throw new Error("The Linux package check requires a Linux host");
}

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const { version } = JSON.parse(readFileSync(join(projectRoot, "package.json"), "utf8"));
const architecture = process.arch === "x64" ? "amd64" : process.arch;
const rpmArchitecture = process.arch === "x64" ? "x86_64" : process.arch === "arm64" ? "aarch64" : process.arch;
const bundleRoot = join(projectRoot, "target", "release", "bundle");
const deb = join(bundleRoot, "deb", `PortMate_${version}_${architecture}.deb`);
const rpm = join(bundleRoot, "rpm", `PortMate-${version}-1.${rpmArchitecture}.rpm`);
const appImage = join(bundleRoot, "appimage", `PortMate_${version}_${architecture}.AppImage`);
const auditRoot = mkdtempSync(join(tmpdir(), "portmate-package-check-"));

try {
  const debRoot = join(auditRoot, "deb");
  run("dpkg-deb", ["-x", deb, debRoot]);
  assertExecutable(join(debRoot, "usr", "bin", "portmate"));
  const debBridge = join(debRoot, "usr", "bin", "portmate-mcp");
  assertExecutable(debBridge);
  assertFile(join(debRoot, "usr", "share", "applications", "PortMate.desktop"));
  assertFile(join(debRoot, "usr", "share", "icons", "hicolor", "32x32", "apps", "portmate.png"));
  assertFile(join(debRoot, "usr", "share", "icons", "hicolor", "128x128", "apps", "portmate.png"));
  assertSameFile(join(projectRoot, "LICENSE"), join(debRoot, "usr", "lib", "PortMate", "LICENSE"));

  const rpmRoot = join(auditRoot, "rpm");
  extractRpm(rpm, rpmRoot);
  assertExecutable(join(rpmRoot, "usr", "bin", "portmate"));
  const rpmBridge = join(rpmRoot, "usr", "bin", "portmate-mcp");
  assertExecutable(rpmBridge);
  assertFile(join(rpmRoot, "usr", "share", "applications", "PortMate.desktop"));
  assertFile(join(rpmRoot, "usr", "share", "icons", "hicolor", "32x32", "apps", "portmate.png"));
  assertFile(join(rpmRoot, "usr", "share", "icons", "hicolor", "128x128", "apps", "portmate.png"));
  assertSameFile(join(projectRoot, "LICENSE"), join(rpmRoot, "usr", "lib", "PortMate", "LICENSE"));

  const appImageRoot = join(auditRoot, "appimage");
  run(appImage, ["--appimage-extract"], { cwd: appImageRoot, quiet: true, createCwd: true });
  const extracted = join(appImageRoot, "squashfs-root");
  assertExecutable(join(extracted, "usr", "bin", "portmate"));
  const appImageBridge = join(extracted, "usr", "bin", "portmate-mcp");
  assertExecutable(appImageBridge);
  assertFile(join(extracted, "PortMate.png"));
  assertSameFile(join(projectRoot, "LICENSE"), join(extracted, "usr", "lib", "PortMate", "LICENSE"));

  for (const [kind, bridge] of [["DEB", debBridge], ["RPM", rpmBridge], ["AppImage", appImageBridge]]) {
    checkPackagedBridge(kind, bridge);
  }

  console.log(JSON.stringify({
    deb,
    rpm,
    appImage,
    verifiedPackages: ["DEB", "RPM", "AppImage"],
    verified: ["main binary", "MCP sidecar", "desktop entry", "icons", "license", "stdio SDK per package", "HTTP SDK per package"],
  }, null, 2));
} finally {
  rmSync(auditRoot, { recursive: true, force: true });
}

function assertFile(path) {
  accessSync(path, constants.R_OK);
  if (!statSync(path).isFile()) throw new Error(`Expected a regular package file: ${path}`);
}

function assertExecutable(path) {
  assertFile(path);
  accessSync(path, constants.X_OK);
}

function assertSameFile(expected, actual) {
  assertFile(expected);
  assertFile(actual);
  if (!readFileSync(expected).equals(readFileSync(actual))) {
    throw new Error(`Package file does not match ${expected}: ${actual}`);
  }
}

function extractRpm(archive, destination) {
  mkdirSync(destination, { recursive: true });
  let payload = spawnSync("rpm2cpio", [archive], {
    encoding: null,
    maxBuffer: 256 * 1024 * 1024,
  });
  let extractor = "rpm2cpio";
  let extractorArgs = [archive];
  if (payload.error?.code === "ENOENT") {
    extractor = "7z";
    extractorArgs = ["x", "-so", archive];
    payload = spawnSync(extractor, extractorArgs, {
      encoding: null,
      maxBuffer: 256 * 1024 * 1024,
    });
  }
  assertCommandResult(extractor, extractorArgs, payload);
  const unpack = spawnSync("cpio", ["-idmu", "--quiet", "--no-absolute-filenames"], {
    cwd: destination,
    input: payload.stdout,
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
  });
  assertCommandResult("cpio", ["-idmu", "--quiet", "--no-absolute-filenames"], unpack);
}

function checkPackagedBridge(kind, bridge) {
  for (const script of ["scripts/mcp-stdio-client-check.mjs", "scripts/mcp-http-client-check.mjs"]) {
    run(process.execPath, [script], {
      cwd: projectRoot,
      env: { ...process.env, PORTMATE_MCP_BINARY: bridge },
    });
  }
  console.log(`${kind} MCP sidecar protocol checks passed`);
}

function assertCommandResult(command, args, result) {
  if (result.error) throw result.error;
  if (result.status !== 0) {
    if (result.stderr) process.stderr.write(result.stderr);
    throw new Error(`${command} ${args.join(" ")} failed with exit code ${result.status}`);
  }
}

function run(command, args, options = {}) {
  if (options.createCwd) mkdirSync(options.cwd, { recursive: true });
  const result = spawnSync(command, args, {
    cwd: options.cwd ?? projectRoot,
    env: options.env ?? process.env,
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
  });
  if (!options.quiet && result.stdout) process.stdout.write(result.stdout);
  if (result.stderr) process.stderr.write(result.stderr);
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed with exit code ${result.status}`);
  }
}
