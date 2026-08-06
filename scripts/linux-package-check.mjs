import {
  accessSync,
  constants,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  readlinkSync,
  rmSync,
  statSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { smokePackagedApplicationLifecycle } from "./native-packaged-smoke.mjs";
import { smokePackagedSidecarParentWatchdog } from "./native-packaged-sidecar-smoke.mjs";

if (process.platform !== "linux") {
  throw new Error("The Linux package check requires a Linux host");
}
if (!process.env.DISPLAY?.trim()) {
  throw new Error("The Linux package check requires an active X11 display in DISPLAY");
}

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const { version } = JSON.parse(readFileSync(join(projectRoot, "package.json"), "utf8"));
const tauriConfig = readJson(join(projectRoot, "src-tauri", "tauri.conf.json"));
const jetBrainsMonoLicense = join(projectRoot, "THIRD_PARTY_LICENSES", "JetBrainsMono-OFL.txt");
const defaultCapability = readJson(join(projectRoot, "src-tauri", "capabilities", "default.json"));
const detachedCapability = readJson(join(projectRoot, "src-tauri", "capabilities", "detached-pane.json"));
const productionCsp = verifyTauriSecurityConfiguration(
  tauriConfig,
  defaultCapability,
  detachedCapability,
);
const architecture = process.arch === "x64" ? "amd64" : process.arch;
const rpmArchitecture = process.arch === "x64" ? "x86_64" : process.arch === "arm64" ? "aarch64" : process.arch;
const bundleRoot = join(projectRoot, "target", "release", "bundle");
const deb = join(bundleRoot, "deb", `PortMate_${version}_${architecture}.deb`);
const rpm = join(bundleRoot, "rpm", `PortMate-${version}-1.${rpmArchitecture}.rpm`);
const appImage = join(bundleRoot, "appimage", `PortMate_${version}_${architecture}.AppImage`);
const auditRoot = mkdtempSync(join(tmpdir(), "portmate package check "));
const runtimeSmokes = [];
const sidecarWatchdogSmokes = [];

try {
  const debRoot = join(auditRoot, "deb");
  run("dpkg-deb", ["-x", deb, debRoot]);
  const debMain = join(debRoot, "usr", "bin", "portmate");
  assertPackagedMain("DEB", debMain, tauriConfig.identifier, productionCsp);
  runtimeSmokes.push(await checkPackagedApplication("DEB", debMain, "runtime-deb"));
  const debBridge = join(debRoot, "usr", "bin", "portmate-mcp");
  assertExecutable(debBridge);
  assertDesktopEntry(join(debRoot, "usr", "share", "applications", "PortMate.desktop"));
  assertFile(join(debRoot, "usr", "share", "icons", "hicolor", "32x32", "apps", "portmate.png"), 0o644);
  assertFile(join(debRoot, "usr", "share", "icons", "hicolor", "128x128", "apps", "portmate.png"), 0o644);
  assertSameFile(join(projectRoot, "LICENSE"), join(debRoot, "usr", "lib", "PortMate", "LICENSE"), 0o644);
  assertSameFile(
    jetBrainsMonoLicense,
    join(debRoot, "usr", "lib", "PortMate", "THIRD_PARTY_LICENSES", "JetBrainsMono-OFL.txt"),
    0o644,
  );
  assertPortableTree(debRoot);

  const rpmRoot = join(auditRoot, "rpm");
  extractRpm(rpm, rpmRoot);
  const rpmMain = join(rpmRoot, "usr", "bin", "portmate");
  assertPackagedMain("RPM", rpmMain, tauriConfig.identifier, productionCsp);
  runtimeSmokes.push(await checkPackagedApplication("RPM", rpmMain, "runtime-rpm"));
  const rpmBridge = join(rpmRoot, "usr", "bin", "portmate-mcp");
  assertExecutable(rpmBridge);
  assertDesktopEntry(join(rpmRoot, "usr", "share", "applications", "PortMate.desktop"));
  assertFile(join(rpmRoot, "usr", "share", "icons", "hicolor", "32x32", "apps", "portmate.png"), 0o644);
  assertFile(join(rpmRoot, "usr", "share", "icons", "hicolor", "128x128", "apps", "portmate.png"), 0o644);
  assertSameFile(join(projectRoot, "LICENSE"), join(rpmRoot, "usr", "lib", "PortMate", "LICENSE"), 0o644);
  assertSameFile(
    jetBrainsMonoLicense,
    join(rpmRoot, "usr", "lib", "PortMate", "THIRD_PARTY_LICENSES", "JetBrainsMono-OFL.txt"),
    0o644,
  );
  assertPortableTree(rpmRoot);

  const appImageRoot = join(auditRoot, "appimage");
  run(appImage, ["--appimage-extract"], { cwd: appImageRoot, quiet: true, createCwd: true });
  const extracted = join(appImageRoot, "squashfs-root");
  const appImageMain = join(extracted, "usr", "bin", "portmate");
  assertPackagedMain("AppImage", appImageMain, tauriConfig.identifier, productionCsp);
  const appImageLauncher = join(extracted, "AppRun");
  assertExecutable(appImageLauncher);
  runtimeSmokes.push(await checkPackagedApplication(
    "AppImage",
    appImageLauncher,
    "runtime-appimage",
  ));
  const appImageBridge = join(extracted, "usr", "bin", "portmate-mcp");
  assertExecutable(appImageBridge);
  assertSameFile(
    join(projectRoot, "src-tauri", "icons", "128x128@2x.png"),
    join(extracted, "PortMate.png"),
    0o644,
  );
  assertDesktopEntry(join(extracted, "usr", "share", "applications", "PortMate.desktop"));
  assertSymlink(join(extracted, ".DirIcon"), "PortMate.png");
  assertSymlink(join(extracted, "PortMate.desktop"), "usr/share/applications/PortMate.desktop");
  assertSymlink(join(extracted, "portmate.png"), "usr/share/icons/hicolor/256x256@2/apps/portmate.png");
  assertSameFile(
    join(projectRoot, "src-tauri", "icons", "128x128@2x.png"),
    join(extracted, "usr", "share", "icons", "hicolor", "256x256@2", "apps", "portmate.png"),
    0o644,
  );
  assertSameFile(join(projectRoot, "LICENSE"), join(extracted, "usr", "lib", "PortMate", "LICENSE"), 0o644);
  assertSameFile(
    jetBrainsMonoLicense,
    join(extracted, "usr", "lib", "PortMate", "THIRD_PARTY_LICENSES", "JetBrainsMono-OFL.txt"),
    0o644,
  );
  assertPortableTree(extracted);

  for (const [kind, bridge] of [["DEB", debBridge], ["RPM", rpmBridge], ["AppImage", appImageBridge]]) {
    checkPackagedBridge(kind, bridge);
    sidecarWatchdogSmokes.push({
      package: kind,
      result: await smokePackagedSidecarParentWatchdog({
        executable: bridge,
        label: `${kind} packaged MCP sidecar`,
      }),
    });
  }

  console.log(JSON.stringify({
    deb,
    rpm,
    appImage,
    verifiedPackages: ["DEB", "RPM", "AppImage"],
    verified: [
      "main binary",
      "MCP sidecar",
      "desktop entry",
      "icons",
      "application and JetBrains Mono licenses",
      "production CSP",
      "main/detached capabilities",
      "portable symlinks and permissions",
      "extraction, execution, runtime data, and sidecar paths containing spaces",
      "packaged main-process IPC, stable restart and legacy-migration Store, fail-closed two-store conflict, credential rotation, clean exit, and endpoint cleanup",
      "TypeScript/Python/Go/Rust/Ruby/Java/Kotlin/C#/Swift stdio SDK per package",
      "TypeScript/Python/Go/Rust/Ruby/Java/Kotlin/C#/Swift HTTP SDK per package",
      "packaged MCP sidecar HTTP readiness and abnormal-parent cleanup",
    ],
    runtimeSmokes,
    sidecarWatchdogSmokes,
  }, null, 2));
} finally {
  rmSync(auditRoot, { recursive: true, force: true });
}

async function checkPackagedApplication(kind, executable, dataRootName) {
  return {
    package: kind,
    result: await smokePackagedApplicationLifecycle({
      executable,
      dataDirectory: join(auditRoot, dataRootName, "dev.portmate.desktop"),
      label: `${kind} packaged application`,
      exitAfterMs: 5_000,
      timeoutMs: 45_000,
    }),
  };
}

function assertFile(path, expectedMode) {
  accessSync(path, constants.R_OK);
  const metadata = lstatSync(path);
  if (!metadata.isFile()) throw new Error(`Expected a regular package file: ${path}`);
  if (expectedMode !== undefined) assertMode(path, metadata, expectedMode);
}

function assertExecutable(path) {
  assertFile(path, 0o755);
  accessSync(path, constants.X_OK);
}

function assertSameFile(expected, actual, expectedMode) {
  assertFile(expected);
  assertFile(actual, expectedMode);
  if (!readFileSync(expected).equals(readFileSync(actual))) {
    throw new Error(`Package file does not match ${expected}: ${actual}`);
  }
}

function assertMode(path, metadata, expectedMode) {
  const actual = metadata.mode & 0o777;
  if (actual !== expectedMode) {
    throw new Error(`Expected mode ${expectedMode.toString(8)} for ${path}, found ${actual.toString(8)}`);
  }
}

function assertSymlink(path, expectedTarget) {
  const metadata = lstatSync(path);
  if (!metadata.isSymbolicLink()) throw new Error(`Expected a symbolic link: ${path}`);
  const target = readlinkSync(path);
  if (target !== expectedTarget) {
    throw new Error(`Expected ${path} -> ${expectedTarget}, found ${target}`);
  }
}

function assertDesktopEntry(path) {
  assertFile(path, 0o644);
  const fields = new Map();
  for (const line of readFileSync(path, "utf8").split(/\r?\n/)) {
    if (!line || line.startsWith("[") || line.startsWith("#")) continue;
    const separator = line.indexOf("=");
    if (separator <= 0) throw new Error(`Invalid desktop entry line in ${path}: ${line}`);
    const key = line.slice(0, separator);
    if (fields.has(key)) throw new Error(`Duplicate desktop entry field ${key}: ${path}`);
    fields.set(key, line.slice(separator + 1));
  }
  const expected = {
    Categories: "Development;",
    Exec: "portmate",
    Icon: "portmate",
    Name: "PortMate",
    StartupWMClass: "portmate",
    Terminal: "false",
    Type: "Application",
  };
  for (const [key, value] of Object.entries(expected)) {
    if (fields.get(key) !== value) {
      throw new Error(`Expected ${key}=${value} in ${path}, found ${fields.get(key)}`);
    }
  }
}

function assertPackagedMain(kind, path, identifier, csp) {
  assertExecutable(path);
  const binary = readFileSync(path);
  for (const marker of [
    identifier,
    csp,
    "pane-*",
    "allow-create-webview-window",
    "allow-set-position",
    "allow-set-size",
  ]) {
    if (!binary.includes(Buffer.from(marker))) {
      throw new Error(`${kind} main binary does not embed required Tauri metadata: ${marker}`);
    }
  }
}

function assertPortableTree(root) {
  for (const entry of readdirSync(root)) visit(join(root, entry));

  function visit(path) {
    const metadata = lstatSync(path);
    if (metadata.isSymbolicLink()) {
      const target = readlinkSync(path);
      if (isAbsolute(target)) throw new Error(`Package contains an absolute symlink: ${path} -> ${target}`);
      const resolvedTarget = resolve(dirname(path), target);
      const fromRoot = relative(root, resolvedTarget);
      if (fromRoot === ".." || fromRoot.startsWith(`..${sep}`) || isAbsolute(fromRoot)) {
        throw new Error(`Package symlink escapes its root: ${path} -> ${target}`);
      }
      statSync(path);
      return;
    }
    if (metadata.isDirectory()) {
      for (const entry of readdirSync(path)) visit(join(path, entry));
      return;
    }
    if (metadata.isFile()) {
      const mode = metadata.mode & 0o777;
      if ((mode & 0o022) !== 0) {
        throw new Error(`Package file is group/world writable (${mode.toString(8)}): ${path}`);
      }
    }
  }
}

function verifyTauriSecurityConfiguration(config, mainCapability, paneCapability) {
  if (config.identifier !== "dev.portmate.desktop") {
    throw new Error(`Unexpected Tauri identifier: ${config.identifier}`);
  }
  if (config.app?.security?.devCsp !== null) {
    throw new Error("Tauri devCsp must remain null so it cannot replace the production policy");
  }
  const csp = config.app?.security?.csp;
  if (typeof csp !== "string") throw new Error("Tauri production CSP is missing");
  const expectedDirectives = {
    "base-uri": ["'none'"],
    "connect-src": ["ipc:", "http://ipc.localhost"],
    "default-src": ["'self'", "customprotocol:", "asset:"],
    "font-src": ["'self'", "data:"],
    "form-action": ["'none'"],
    "frame-src": ["'none'"],
    "img-src": ["'self'", "asset:", "http://asset.localhost", "data:", "blob:"],
    "object-src": ["'none'"],
    "script-src": ["'self'"],
    "style-src": ["'self'", "'unsafe-inline'"],
  };
  const directives = parseCsp(csp);
  if (directives.size !== Object.keys(expectedDirectives).length) {
    throw new Error(`Unexpected production CSP directive count: ${directives.size}`);
  }
  for (const [directive, expectedValues] of Object.entries(expectedDirectives)) {
    assertExactArray(`CSP ${directive}`, directives.get(directive), expectedValues);
  }
  assertExactArray("main capability windows", mainCapability.windows, ["main"]);
  assertExactArray("main capability permissions", mainCapability.permissions, [
    "core:default",
    "core:webview:allow-create-webview-window",
    "core:window:allow-set-position",
    "core:window:allow-set-size",
    "core:window:allow-show",
  ]);
  assertExactArray("detached capability windows", paneCapability.windows, ["pane-*"]);
  assertExactArray("detached capability permissions", paneCapability.permissions, [
    "core:default",
    "core:window:allow-close",
  ]);
  return csp;
}

function parseCsp(csp) {
  const directives = new Map();
  for (const part of csp.split(";")) {
    const fields = part.trim().split(/\s+/).filter(Boolean);
    if (!fields.length) continue;
    const [name, ...values] = fields;
    if (directives.has(name)) throw new Error(`Duplicate CSP directive: ${name}`);
    directives.set(name, values);
  }
  return directives;
}

function assertExactArray(label, actual, expected) {
  if (!Array.isArray(actual) || actual.length !== expected.length
    || actual.some((value, index) => value !== expected[index])) {
    throw new Error(`${label} does not match the release policy: ${JSON.stringify(actual)}`);
  }
}

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
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
  run(process.execPath, ["scripts/mcp-typescript-client-check.mjs"], {
    cwd: projectRoot,
    env: { ...process.env, PORTMATE_MCP_BINARY: bridge },
  });
  run(process.execPath, ["scripts/mcp-python-client-check.mjs"], {
    cwd: projectRoot,
    env: { ...process.env, PORTMATE_MCP_BINARY: bridge },
  });
  run(process.execPath, ["scripts/mcp-go-client-check.mjs"], {
    cwd: projectRoot,
    env: { ...process.env, PORTMATE_MCP_BINARY: bridge },
  });
  run(process.execPath, ["scripts/mcp-rust-client-check.mjs"], {
    cwd: projectRoot,
    env: { ...process.env, PORTMATE_MCP_BINARY: bridge },
  });
  run(process.execPath, ["scripts/mcp-ruby-client-check.mjs"], {
    cwd: projectRoot,
    env: { ...process.env, PORTMATE_MCP_BINARY: bridge },
  });
  run(process.execPath, ["scripts/mcp-java-client-check.mjs"], {
    cwd: projectRoot,
    env: { ...process.env, PORTMATE_MCP_BINARY: bridge },
  });
  run(process.execPath, ["scripts/mcp-kotlin-client-check.mjs"], {
    cwd: projectRoot,
    env: { ...process.env, PORTMATE_MCP_BINARY: bridge },
  });
  run(process.execPath, ["scripts/mcp-csharp-client-check.mjs"], {
    cwd: projectRoot,
    env: { ...process.env, PORTMATE_MCP_BINARY: bridge },
  });
  run(process.execPath, ["scripts/mcp-swift-client-check.mjs"], {
    cwd: projectRoot,
    env: { ...process.env, PORTMATE_MCP_BINARY: bridge },
  });
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
